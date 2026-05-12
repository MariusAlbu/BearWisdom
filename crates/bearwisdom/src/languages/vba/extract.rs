// =============================================================================
// languages/vba/extract.rs  —  VBA (Visual Basic for Applications) extractor
//
// No tree-sitter grammar — uses a line scanner (case-insensitive).
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — Sub / Function / [Public|Private|Friend] Sub|Function
//   Function  — Declare [PtrSafe] Function/Sub (Win32 / COM API declarations)
//   Class     — Class_Initialize (VBA class module marker) / Attribute VB_Name
//   Property  — Property Get / Property Let / Property Set
//   Variable  — Dim / Public / Private at module scope
//
// REFERENCES:
//   Calls     — <SubName> [args]  or  Call <SubName>
//
// VBA quirks:
//   - Keywords are case-insensitive.
//   - End Sub / End Function closes a scope.
//   - Attribute VB_Name = "<name>" is the class name in .cls files.
//   - Continuation: " _" at end of line — the next physical line continues
//     the same logical line and must not be treated as an independent statement.
//   - Conditional compilation: #If / #ElseIf / #Else / #End If directives
//     wrap platform-specific code. Both branches are extracted so that Declare
//     statements work regardless of which branch the VBA compiler selects.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Track current enclosing Sub/Function for nested Calls.
    let mut current_proc: Option<usize> = None;
    // Track whether we are inside a Sub/Function body.
    let mut in_proc = false;
    let mut class_name: Option<String> = None;
    // True when the previous logical line ended with the continuation marker " _".
    // A continuation line is part of the preceding logical line — it must not
    // be scanned for independent call statements.
    let mut prev_continues = false;

    for (lineno, raw_line) in source.lines().enumerate() {
        let row = lineno as u32;
        let line = raw_line.trim();

        // Skip empty lines and comments.
        if line.is_empty() || line.starts_with('\'') || line.to_uppercase().starts_with("REM ") {
            // An empty / comment line does not reset the continuation flag — a
            // real continuation is `<expr> _\n<continuation>` with no blanks
            // between them; blanks here belong to completely separate statements.
            prev_continues = false;
            continue;
        }

        // Conditional compilation directives (#If / #ElseIf / #Else / #End If)
        // are structural markers, not statements. Skip the directive line itself
        // but continue processing the code that follows it so that Declare
        // statements and other declarations inside any branch are extracted.
        let upper = line.to_uppercase();
        if upper.starts_with("#IF ")
            || upper.starts_with("#ELSEIF ")
            || upper == "#ELSE"
            || upper.starts_with("#ELSE ")
            || upper == "#END IF"
            || upper.starts_with("#END IF ")
            || upper == "#ENDIF"
        {
            prev_continues = false;
            continue;
        }

        // Detect whether this line ends with a continuation marker.
        // The marker is " _" (space + underscore) at end of the trimmed line.
        let this_continues = line.ends_with(" _") || line.ends_with("\t_");

        // If the previous line continued into this one, this line is not an
        // independent statement — skip call detection entirely.
        let is_continuation = prev_continues;

        // Update the continuation flag now, before any `continue` branches
        // below. Each early exit (symbol extraction, proc markers, etc.) is a
        // complete line that was fully handled — the continuation state for the
        // NEXT iteration must reflect whether THIS line ended with " _".
        prev_continues = this_continues;

        // Attribute VB_Name = "ClassName"  →  Class symbol (first occurrence).
        if class_name.is_none() {
            if let Some(name) = parse_vb_name(line) {
                class_name = Some(name.clone());
                let idx = symbols.len();
                symbols.push(make_symbol(
                    name.clone(),
                    name,
                    SymbolKind::Class,
                    row,
                    None,
                    None,
                ));
                // Class is the implicit parent of everything.
                if current_proc.is_none() {
                    current_proc = Some(idx);
                }
                continue;
            }
        }

        // Sub or Function start.
        if let Some((name, kind)) = parse_proc_start(&upper, line) {
            let parent = if class_name.is_some() {
                symbols.iter().position(|s| s.kind == SymbolKind::Class)
            } else {
                None
            };
            let sig = line.to_string();
            let idx = symbols.len();
            symbols.push(make_symbol(
                name.clone(),
                name,
                kind,
                row,
                Some(sig),
                parent,
            ));
            current_proc = Some(idx);
            in_proc = true;
            continue;
        }

        // Declare [PtrSafe] Function/Sub — Win32 / COM API declaration.
        // These are always module-scope (never inside a proc body) and are
        // treated as callable Function symbols so that call-site refs resolve.
        if let Some(name) = parse_declare(&upper, line) {
            let parent = if class_name.is_some() {
                symbols.iter().position(|s| s.kind == SymbolKind::Class)
            } else {
                None
            };
            symbols.push(make_symbol(
                name.clone(),
                name,
                SymbolKind::Function,
                row,
                Some(line.to_string()),
                parent,
            ));
            continue;
        }

        // Property Get / Let / Set.
        if let Some(name) = parse_property(&upper, line) {
            let parent = if class_name.is_some() {
                symbols.iter().position(|s| s.kind == SymbolKind::Class)
            } else {
                None
            };
            let sig = line.to_string();
            let idx = symbols.len();
            symbols.push(make_symbol(
                name.clone(),
                name,
                SymbolKind::Property,
                row,
                Some(sig),
                parent,
            ));
            current_proc = Some(idx);
            in_proc = true;
            continue;
        }

        // End Sub / End Function / End Property.
        if upper == "END SUB"
            || upper == "END FUNCTION"
            || upper == "END PROPERTY"
            || upper.starts_with("END SUB ")
            || upper.starts_with("END FUNCTION ")
        {
            in_proc = false;
            // current_proc stays so we can attribute trailing refs; reset at next proc.
            continue;
        }

        // Module-level variable declarations (only outside proc bodies, or top-level Dim).
        if !in_proc {
            if let Some(name) = parse_variable_decl(&upper, line) {
                symbols.push(make_symbol(
                    name.clone(),
                    name,
                    SymbolKind::Variable,
                    row,
                    Some(line.to_string()),
                    None,
                ));
                continue;
            }
        }

        // Call statements inside proc bodies — only on non-continuation lines.
        if in_proc && !is_continuation {
            let source_idx = current_proc.unwrap_or(0);
            if let Some(target) = parse_call_stmt(&upper, line) {
                refs.push(ExtractedRef {
                    source_symbol_index: source_idx,
                    target_name: target,
                    kind: EdgeKind::Calls,
                    line: row,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        }
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// Parse "Attribute VB_Name = \"ClassName\"" → Some("ClassName")
fn parse_vb_name(line: &str) -> Option<String> {
    let upper = line.to_uppercase();
    if !upper.starts_with("ATTRIBUTE VB_NAME") {
        return None;
    }
    // Find the string after '='.
    let eq_pos = line.find('=')?;
    let rest = line[eq_pos + 1..].trim();
    let name = rest.trim_matches('"').to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Parse Sub/Function header: optionally preceded by Public/Private/Friend/Static.
/// Returns (name, SymbolKind::Function).
fn parse_proc_start(upper: &str, original: &str) -> Option<(String, SymbolKind)> {
    // Strip visibility modifier.
    let body = strip_visibility_prefix(upper);

    let (keyword, rest) = if body.starts_with("SUB ") {
        ("SUB", &body[4..])
    } else if body.starts_with("FUNCTION ") {
        ("FUNCTION", &body[9..])
    } else {
        return None;
    };
    let _ = keyword;

    // Name is everything up to '(' or whitespace.
    let name = rest
        .split(|c| c == '(' || c == ' ')
        .next()?
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }

    // Recover original-case name.
    let original_body = strip_visibility_prefix_original(original);
    let name_original = original_body
        .split_whitespace()
        .nth(1)
        .unwrap_or(&name)
        .split('(')
        .next()
        .unwrap_or(&name)
        .to_string();

    Some((name_original, SymbolKind::Function))
}

/// Parse "Declare [PtrSafe] Function/Sub <Name> Lib ..." → Some(name)
///
/// VBA Declare statements introduce a callable symbol bound to an external
/// DLL entry point. Both branches of a #If VBA7 / #Else block use the same
/// name, so extracting from all branches is harmless — duplicate names unify
/// in the symbol index under the same qualified path.
fn parse_declare(upper: &str, original: &str) -> Option<String> {
    // Strip optional visibility prefix.
    let body = strip_visibility_prefix(upper);
    if !body.starts_with("DECLARE ") {
        return None;
    }
    let after_declare = body[8..].trim_start();

    // Optional PtrSafe keyword (VBA7 / 64-bit).
    let after_ptrsafe = if after_declare.starts_with("PTRSAFE ") {
        &after_declare[8..]
    } else {
        after_declare
    };

    // Must be followed by FUNCTION or SUB.
    let after_keyword = if after_ptrsafe.starts_with("FUNCTION ") {
        &after_ptrsafe[9..]
    } else if after_ptrsafe.starts_with("SUB ") {
        &after_ptrsafe[4..]
    } else {
        return None;
    };

    // Name is the first token before '(' or whitespace.
    let name_upper = after_keyword
        .split(|c| c == '(' || c == ' ')
        .next()?
        .trim()
        .to_string();
    if name_upper.is_empty() {
        return None;
    }

    // Recover original-case name from the original (non-uppercased) line.
    // Walk the original tokens in parallel to the upper tokens.
    let orig_body = strip_visibility_prefix_original(original);
    let orig_name = recover_declare_name(orig_body, &name_upper);
    Some(orig_name)
}

/// Recover original-case name from a Declare line given the uppercased name.
fn recover_declare_name(orig_body: &str, _name_upper: &str) -> String {
    // Skip "Declare", optional "PtrSafe", "Function"/"Sub", then take the name.
    let mut tokens = orig_body.split_whitespace();
    // "Declare"
    tokens.next();
    let next = tokens.next().unwrap_or("");
    // Optional "PtrSafe"
    let keyword_tok = if next.to_uppercase() == "PTRSAFE" {
        tokens.next().unwrap_or("")
    } else {
        next
    };
    let kw_upper = keyword_tok.to_uppercase();
    if kw_upper == "FUNCTION" || kw_upper == "SUB" {
        let name_tok = tokens.next().unwrap_or("");
        name_tok.split('(').next().unwrap_or(name_tok).to_string()
    } else {
        _name_upper.to_string()
    }
}

/// Parse "Property Get/Let/Set <Name>" → Some(name)
fn parse_property(upper: &str, original: &str) -> Option<String> {
    let body = strip_visibility_prefix(upper);
    if !body.starts_with("PROPERTY ") {
        return None;
    }
    // "PROPERTY GET Name(...)"
    let rest = &body[9..]; // after "PROPERTY "
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }
    // tokens[0] = GET/LET/SET, tokens[1] = name
    let name_upper = tokens[1].split('(').next().unwrap_or("").to_string();
    if name_upper.is_empty() {
        return None;
    }
    // Recover original case from original line.
    let orig_body = strip_visibility_prefix_original(original);
    let orig_tokens: Vec<&str> = orig_body.split_whitespace().collect();
    let name = if orig_tokens.len() >= 3 {
        orig_tokens[2].split('(').next().unwrap_or(&name_upper).to_string()
    } else {
        name_upper
    };
    Some(name)
}

/// Parse module-scope variable declarations: Dim/Public/Private <name> [As <type>]
fn parse_variable_decl(upper: &str, original: &str) -> Option<String> {
    let keyword = if upper.starts_with("DIM ") {
        "DIM"
    } else if upper.starts_with("PUBLIC ") && !upper.contains("SUB ") && !upper.contains("FUNCTION ") {
        "PUBLIC"
    } else if upper.starts_with("PRIVATE ") && !upper.contains("SUB ") && !upper.contains("FUNCTION ") {
        "PRIVATE"
    } else {
        return None;
    };

    let rest = &original[keyword.len()..].trim_start();
    let name = rest
        .split(|c| c == ' ' || c == ',' || c == '\t')
        .next()?
        .trim()
        .to_string();
    if name.is_empty() || name.to_uppercase() == "AS" {
        return None;
    }
    Some(name)
}

/// Parse a call statement: "Call <Name>" or "<Name> [args]" inside a proc body.
/// Returns the callee name if it looks like a procedure call.
fn parse_call_stmt(upper: &str, original: &str) -> Option<String> {
    // Explicit "Call SubName" or "Call Obj.Method"
    if upper.starts_with("CALL ") {
        let rest = &original[5..].trim_start();
        // Split on space or '(' to get the callee token, then take only the
        // simple name (before any '.') to avoid emitting "Obj.Method" chains
        // as a single opaque identifier.
        let token = rest.split(|c| c == ' ' || c == '(').next()?.trim();
        // Use the last segment of a dotted chain (e.g. "Create.protInit" → "protInit").
        let name = token.split('.').last().unwrap_or(token).to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }

    // Implicit call: first token is an identifier that looks like a procedure name,
    // not a keyword. This is heuristic — we only emit for identifiers that start
    // uppercase and aren't obviously assignments or control flow.
    let first_token = original.split_whitespace().next().unwrap_or("");
    if first_token.is_empty()
        || first_token.starts_with('"')   // VBA string literal (`"Set-Cookie", val`)
        || first_token.starts_with('\'')  // VBA comment / single-quoted string
        || first_token.contains('=')
        || first_token.contains('.')
        || first_token.contains('(')
        || first_token.contains(',')   // trailing comma → this token is an argument
        || first_token.contains('/')   // path-like string
        || first_token.contains('\\')  // path-like string
    {
        return None;
    }
    let ft_upper = first_token.to_uppercase();
    if is_vba_keyword(&ft_upper) {
        return None;
    }
    // Only emit if line doesn't look like an assignment.
    if upper.contains(" = ") {
        return None;
    }
    // Must have arguments (space after name or parentheses) to distinguish from labels.
    if !upper.contains(' ') && !upper.contains('(') {
        return None;
    }
    // Strip trailing punctuation that can cling to an identifier (e.g. "QuoteChar,").
    let raw = first_token.split('(').next().unwrap_or(first_token);
    let name = raw.trim_end_matches(|c: char| c == ',' || c == ')' || c == ';' || c == ':' || c.is_whitespace());
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn strip_visibility_prefix<'a>(upper: &'a str) -> &'a str {
    for prefix in &["PUBLIC ", "PRIVATE ", "FRIEND ", "STATIC ", "PROTECTED "] {
        if upper.starts_with(prefix) {
            return &upper[prefix.len()..];
        }
    }
    upper
}

fn strip_visibility_prefix_original<'a>(original: &'a str) -> &'a str {
    let upper = original.to_uppercase();
    for prefix in &["PUBLIC ", "PRIVATE ", "FRIEND ", "STATIC ", "PROTECTED "] {
        if upper.starts_with(prefix) {
            return &original[prefix.len()..];
        }
    }
    original
}

fn is_vba_keyword(upper: &str) -> bool {
    matches!(
        upper,
        "DIM" | "SET" | "LET" | "IF" | "ELSE" | "ELSEIF" | "END" | "FOR" | "NEXT"
        | "DO" | "LOOP" | "WHILE" | "WEND" | "SELECT" | "CASE" | "WITH"
        | "EXIT" | "GOTO" | "RESUME" | "ON" | "ERROR" | "RETURN"
        | "REM" | "OPTION" | "EXPLICIT" | "BASE" | "COMPARE"
        | "ME" | "NEW" | "NOT" | "AND" | "OR" | "XOR" | "IS" | "LIKE"
        | "MOD" | "NOTHING" | "EMPTY" | "NULL" | "TRUE" | "FALSE"
        | "MSGBOX" | "INPUTBOX" | "PRINT" | "DEBUG" | "OPEN" | "CLOSE"
        | "GET" | "PUT" | "SEEK" | "WRITE" | "INPUT" | "LINE"
        | "REDIM" | "ERASE" | "STOP"
        // Event / flow control statements
        | "RAISEEVENT" | "GOSUB" | "DOEVENTS" | "APPEND"
        // Static local variable declarations (modifier inside proc body)
        | "STATIC"
        // I/O and system statements
        | "SAVESETTING" | "DELETESETTING" | "SENDKEYS" | "BEEP"
        // Type / declare-adjacent keywords that appear as statement starters
        | "DECLARE" | "IMPLEMENTS" | "ATTRIBUTE"
    )
}

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    line: u32,
    signature: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
    }
}
