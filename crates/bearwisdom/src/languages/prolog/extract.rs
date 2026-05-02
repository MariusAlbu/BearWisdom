// =============================================================================
// languages/prolog/extract.rs  —  Prolog extractor (line scanner)
//
// No tree-sitter grammar — uses a clause-aware line scanner.
//
// What we extract
// ---------------
// SYMBOLS:
//   Function  — predicates: `functor/arity` pattern
//               Both facts (head.) and rules (head :- body.) are supported.
//   Namespace — `:- module(Name, Exports).`
//
// REFERENCES:
//   Imports   — `:- use_module(library(name)).` or `:- use_module(path).`
//   Calls     — goals in rule bodies (best-effort: immediate goals after `:-`)
//
// Prolog syntax notes:
//   - Clauses end with '.'.
//   - Rules: `head :- body.`
//   - Facts: `head.`
//   - Directives: `:- directive(args).`
//   - Comments: `%` line comments, `/* ... */` block comments.
//   - Functor/arity notation: `append/3`, `member/2`.
//
// This scanner is line-oriented and handles multi-line clauses only in the
// simple case where each clause occupies one logical line (ended with '.').
// Multi-line clauses spanning several source lines are handled by accumulating
// until a '.' terminator is found.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use crate::types::ExtractionResult;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // Accumulate logical lines (clauses can span multiple source lines).
    let mut clause_buf = String::new();
    let mut clause_start_line: u32 = 0;
    let mut in_block_comment = false;

    for (lineno, raw_line) in source.lines().enumerate() {
        let row = lineno as u32;

        // Handle block comments.
        let line = if in_block_comment {
            if let Some(end) = raw_line.find("*/") {
                in_block_comment = false;
                &raw_line[end + 2..]
            } else {
                continue;
            }
        } else {
            raw_line
        };

        // Strip line comments and check for block comment start.
        let line = strip_line_comment(line);
        let (line, starts_block_comment) = strip_block_comment_start(line);
        if starts_block_comment {
            in_block_comment = true;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if clause_buf.is_empty() {
            clause_start_line = row;
        }

        // Accumulate until we hit a '.' that terminates a clause.
        clause_buf.push(' ');
        clause_buf.push_str(trimmed);

        // A clause ends at a '.' not inside quotes and not part of a float.
        if clause_ends(&clause_buf) {
            let clause = clause_buf.trim().to_string();
            clause_buf.clear();
            process_clause(&clause, clause_start_line, &mut symbols, &mut refs);
        }
    }

    // Handle any unterminated final clause.
    if !clause_buf.is_empty() {
        let clause = clause_buf.trim().to_string();
        process_clause(&clause, clause_start_line, &mut symbols, &mut refs);
    }

    ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Clause processor
// ---------------------------------------------------------------------------

fn process_clause(
    clause: &str,
    line: u32,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let clause = clause.trim().trim_end_matches('.').trim();

    // Directive: :- directive(args)
    if clause.starts_with(":-") {
        let body = clause[2..].trim();
        process_directive(body, line, symbols, refs);
        return;
    }

    // Rule: Head :- Body
    // Fact: Head
    //
    // Symbol shape: name=bare functor (`member`), qualified_name=functor/arity
    // (`member/2`). The bare name lets the shared resolver's
    // `lookup.by_name(target)` match a call like `member(X, L)` against the
    // definition without the call site needing to count arity. The fully
    // qualified `functor/arity` form preserves Prolog's predicate identity
    // for downstream tools that need it (e.g., bw_symbol_info disambiguating
    // `member/2` from `member/3`).
    if let Some(neck_pos) = find_neck(clause) {
        let head = clause[..neck_pos].trim();
        let body = clause[neck_pos + 2..].trim(); // skip ":-"
        if let Some((functor, arity)) = extract_predicate(head) {
            let idx = symbols.len();
            symbols.push(make_symbol(
                functor.clone(),
                format!("{}/{}", functor, arity),
                SymbolKind::Function,
                line,
                Some(head.to_string()),
            ));
            // Extract goal calls from body.
            extract_body_goals(body, line, idx, refs);
        }
    } else {
        // Fact (no :-)
        if let Some((functor, arity)) = extract_predicate(clause) {
            symbols.push(make_symbol(
                functor.clone(),
                format!("{}/{}", functor, arity),
                SymbolKind::Function,
                line,
                Some(clause.to_string()),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Directive processor
// ---------------------------------------------------------------------------

fn process_directive(
    body: &str,
    line: u32,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let source_idx = symbols.len().saturating_sub(1);

    // module(Name, Exports) → Namespace
    if body.starts_with("module(") {
        if let Some(name) = extract_first_arg(&body[7..]) {
            symbols.push(make_symbol(
                name.clone(),
                name,
                SymbolKind::Namespace,
                line,
                Some(format!(":- {}", body)),
            ));
        }
        return;
    }

    // use_module(library(name)) or use_module(path)
    if body.starts_with("use_module(") {
        let inner = &body[11..].trim_end_matches(')');
        let module_name = if inner.starts_with("library(") {
            inner[8..].trim_end_matches(')').to_string()
        } else {
            // Path: strip surrounding quotes if present.
            inner
                .trim_matches('\'')
                .trim_matches('"')
                .to_string()
        };
        if !module_name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: module_name.clone(),
                kind: EdgeKind::Imports,
                line,
                module: Some(module_name),
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
        return;
    }

    // ensure_loaded(library(name)) / ensure_loaded(path)
    if body.starts_with("ensure_loaded(") {
        let inner = &body[14..].trim_end_matches(')');
        let module_name = if inner.starts_with("library(") {
            inner[8..].trim_end_matches(')').to_string()
        } else {
            inner.trim_matches('\'').trim_matches('"').to_string()
        };
        if !module_name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: module_name.clone(),
                kind: EdgeKind::Imports,
                line,
                module: Some(module_name),
                chain: None,
                byte_offset: 0,
                            namespace_segments: Vec::new(),
});
        }
        return;
    }
}

// ---------------------------------------------------------------------------
// Goal extraction from rule bodies
// ---------------------------------------------------------------------------

fn extract_body_goals(
    body: &str,
    line: u32,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Split on ',' and ';' (conjunction and disjunction) at the top level
    // (not inside parentheses).
    let goals = split_goals(body);
    for goal in goals {
        let goal = goal.trim();
        if goal.is_empty() {
            continue;
        }
        let functor = goal.split('(').next().unwrap_or(goal).trim();
        if functor.is_empty() {
            continue;
        }
        // Skip variables (start with uppercase or _).
        if functor.starts_with(|c: char| c.is_uppercase() || c == '_') {
            continue;
        }
        // Skip Prolog syntactic operators / control constructs that aren't
        // user-callable predicates. Named stdlib predicates (`member`,
        // `append`, `format`, `findall`, ...) DO get emitted — they resolve
        // against symbols indexed by the prolog-runtime ecosystem when an
        // SWI-Prolog source tree is reachable.
        if is_prolog_operator(functor) {
            continue;
        }
        refs.push(ExtractedRef {
            source_symbol_index: source_idx,
            target_name: functor.to_string(),
            kind: EdgeKind::Calls,
            line,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Find the position of the neck operator `:-` at the top level of the clause
/// (not inside parentheses or functor args).
fn find_neck(clause: &str) -> Option<usize> {
    let bytes = clause.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i + 1 < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b':' if depth == 0 && bytes[i + 1] == b'-' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Extract the functor and arity from a predicate head.
/// "append(H, T, L)" → Some(("append", 3))
/// "foo" → Some(("foo", 0))
fn extract_predicate(head: &str) -> Option<(String, usize)> {
    let head = head.trim();
    if head.is_empty() {
        return None;
    }
    // Skip variables (uppercase start or _).
    if head.starts_with(|c: char| c.is_uppercase() || c == '_') {
        return None;
    }
    // Operators used as predicates: skip.
    if head.starts_with(|c: char| !c.is_alphanumeric() && c != '_' && c != '\'') {
        return None;
    }

    if let Some(paren) = head.find('(') {
        let functor = head[..paren].trim().to_string();
        let args_str = &head[paren + 1..];
        let arity = count_top_level_args(args_str);
        if functor.is_empty() {
            return None;
        }
        Some((functor, arity))
    } else {
        // Atom with no args.
        let functor = head.trim_matches('\'').to_string();
        if functor.is_empty() {
            return None;
        }
        Some((functor, 0))
    }
}

/// Count top-level comma-separated arguments in the args portion
/// (everything after the opening paren, including the closing paren).
fn count_top_level_args(args: &str) -> usize {
    if args.trim().trim_end_matches(')').trim().is_empty() {
        return 0;
    }
    let mut count = 1;
    let mut depth = 0i32;
    for ch in args.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

/// Extract the first argument of a functor: "Name, Exports)" → Some("Name")
fn extract_first_arg(rest: &str) -> Option<String> {
    let arg = rest
        .split(|c| c == ',' || c == ')')
        .next()?
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string();
    if arg.is_empty() { None } else { Some(arg) }
}

/// Split a goal conjunction at top-level ',' and ';'.
fn split_goals(body: &str) -> Vec<&str> {
    let mut goals = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b',' | b';' if depth == 0 => {
                goals.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < body.len() {
        goals.push(&body[start..]);
    }
    goals
}

/// Returns true if a clause's current buffer ends a clause.
/// A clause ends when we see a '.' that is:
/// - not inside quotes
/// - not followed by a digit (float 3.14)
/// - at the end of the accumulated buffer (possibly with trailing whitespace)
fn clause_ends(buf: &str) -> bool {
    let trimmed = buf.trim_end();
    trimmed.ends_with('.')
}

fn strip_line_comment(line: &str) -> &str {
    if let Some(pos) = line.find('%') {
        // Make sure it's not inside a quoted atom.
        let before = &line[..pos];
        let quotes: usize = before.chars().filter(|&c| c == '\'').count();
        if quotes % 2 == 0 {
            return &line[..pos];
        }
    }
    line
}

fn strip_block_comment_start(line: &str) -> (&str, bool) {
    if let Some(pos) = line.find("/*") {
        if let Some(end) = line[pos + 2..].find("*/") {
            // Inline block comment — strip it (ignore content after closing `*/`).
            let _ = end;
            return (&line[..pos], false);
        } else {
            return (&line[..pos], true);
        }
    }
    (line, false)
}

/// Prolog operators and control constructs that aren't callable predicates
/// in the symbol-graph sense — they're built into the engine's evaluator and
/// can never be redefined as a user/library predicate. Emitting them as
/// Calls refs would always produce phantom unresolved targets.
///
/// Named stdlib predicates (`member`, `append`, `format`, `findall`,
/// `assertz`, `nl`, `write`, `read`, ...) are NOT in this list — they resolve
/// against symbols indexed by the prolog-runtime ecosystem.
fn is_prolog_operator(functor: &str) -> bool {
    matches!(
        functor,
        // Term equality / unification / arithmetic comparison
        "=" | "\\=" | "==" | "\\==" | "@<" | "@>" | "@=<" | "@>=" |
        "<" | ">" | ">=" | "=<" | "=:=" | "=\\=" |
        // Arithmetic evaluation
        "is" |
        // Control constructs
        "->" | ";" | "," | "!" | "\\+" |
        // Engine-level constants
        "true" | "false"
    )
}

// ---------------------------------------------------------------------------
// Symbol factory
// ---------------------------------------------------------------------------

fn make_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    line: u32,
    signature: Option<String>,
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
        parent_index: None,
    }
}
