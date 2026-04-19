// =============================================================================
// languages/zig/extract.rs  —  Zig extractor (no grammar)
//
// What we extract
// ---------------
// SYMBOLS:
//   Function    — `fn name(...)` (pub → Public, else Private)
//   Struct      — `const Name = struct { ... }`
//   Enum        — `const Name = enum { ... }`
//   Struct      — `const Name = union { ... }` (tagged unions)
//   Enum        — `const Name = error { ... }` (error sets)
//   Variable    — `const/var name = ...` (plain, non-container)
//   Test        — `test "name" { ... }` / `test name { ... }`
//   Field       — fields inside struct/union blocks
//   EnumMember  — values inside enum/error blocks
//
// REFERENCES:
//   Imports     — `const name = @import("path")` → module path
//   Calls       — function calls on the same line as a declaration
//                 (best-effort: any `ident(` pattern in function bodies)
//
// No tree-sitter grammar. This is a single-pass line scanner with brace depth
// tracking to handle multi-line blocks.
//
// Limitations:
// - Anonymous inline struct/enum expressions are not extracted as named types.
// - Deeply nested comptime generics may not be fully resolved.
// - Method detection relies on being inside a struct/union block (approximated
//   by the parent kind tracking stack).
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

/// Zig primitive types — skip TypeRef for these.
const PRIMITIVES: &[&str] = &[
    "bool", "void", "noreturn", "type", "anyerror", "anyframe", "anytype",
    "comptime_int", "comptime_float",
    "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize",
    "f16", "f32", "f64", "f80", "f128",
    "c_short", "c_int", "c_long", "c_longlong",
    "c_ushort", "c_uint", "c_ulong", "c_ulonglong",
    "c_char", "c_longdouble",
];

/// Zig keywords and built-ins that should not be treated as call targets.
const ZIG_KEYWORDS: &[&str] = &[
    "if", "else", "while", "for", "switch", "return", "break", "continue",
    "defer", "errdefer", "try", "catch", "orelse", "and", "or", "not",
    "const", "var", "comptime", "pub", "extern", "export", "inline",
    "packed", "align", "noalias", "volatile", "allowzero", "noinline",
    "async", "await", "suspend", "nosuspend", "anytype", "usingnamespace",
    "test", "fn", "struct", "enum", "union", "error", "opaque",
    "true", "false", "null", "undefined",
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum ContainerKind {
    Struct,
    Enum,
    Union,
    Error,
    None,
}

pub fn extract(source: &str) -> crate::types::ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim();

        // Skip blank lines and comments
        if trimmed.is_empty() || trimmed.starts_with("//") {
            i += 1;
            continue;
        }

        // Collect preceding doc comments (/// lines)
        let doc = collect_doc_comments(&lines, i);

        // --- Test declaration ---
        if let Some(test_name) = parse_test_declaration(trimmed) {
            let start_line = i as u32;
            let (end_line, _) = skip_block(&lines, i);
            symbols.push(ExtractedSymbol {
                name: test_name.clone(),
                qualified_name: test_name.clone(),
                kind: SymbolKind::Test,
                visibility: None,
                start_line,
                end_line,
                start_col: 0,
                end_col: 0,
                signature: Some(format!("test \"{test_name}\"")),
                doc_comment: doc,
                scope_path: None,
                parent_index: None,
            });
            i = end_line as usize + 1;
            continue;
        }

        // --- Function declaration ---
        if let Some((fn_name, is_pub, signature)) = parse_fn_declaration(trimmed) {
            let start_line = i as u32;
            let (end_line, body_lines) = skip_block_collecting(&lines, i);
            let fn_idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: fn_name.clone(),
                qualified_name: fn_name.clone(),
                kind: SymbolKind::Function,
                visibility: Some(if is_pub { Visibility::Public } else { Visibility::Private }),
                start_line,
                end_line,
                start_col: 0,
                end_col: 0,
                signature: Some(signature),
                doc_comment: doc,
                scope_path: None,
                parent_index: None,
            });
            // Scan body lines for call expressions
            extract_calls_from_body(&body_lines, fn_idx, start_line + 1, &mut refs);
            // Deep-scan the body for anonymous struct blocks (e.g. `return struct { ... }`,
            // `=> struct { ... }`) that contain nested fn/method declarations.
            extract_anon_struct_fns(&body_lines, fn_idx, &mut symbols, &mut refs);
            i = end_line as usize + 1;
            continue;
        }

        // --- const/var declaration ---
        if let Some((decl_name, is_pub, container, import_path)) =
            parse_var_declaration(trimmed)
        {
            let start_line = i as u32;

            // @import case
            if let Some(path) = import_path {
                let decl_idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    name: decl_name.clone(),
                    qualified_name: decl_name.clone(),
                    kind: SymbolKind::Variable,
                    visibility: Some(if is_pub { Visibility::Public } else { Visibility::Private }),
                    start_line,
                    end_line: start_line,
                    start_col: 0,
                    end_col: 0,
                    signature: Some(format!("const {decl_name} = @import(\"{path}\")")),
                    doc_comment: doc,
                    scope_path: None,
                    parent_index: None,
                });
                refs.push(ExtractedRef {
                    source_symbol_index: decl_idx,
                    target_name: path,
                    kind: EdgeKind::Imports,
                    line: start_line,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
                i += 1;
                continue;
            }

            match container {
                ContainerKind::Struct | ContainerKind::Union => {
                    let (end_line, body_lines) = skip_block_collecting(&lines, i);
                    let parent_idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        name: decl_name.clone(),
                        qualified_name: decl_name.clone(),
                        kind: SymbolKind::Struct,
                        visibility: Some(if is_pub { Visibility::Public } else { Visibility::Private }),
                        start_line,
                        end_line,
                        start_col: 0,
                        end_col: 0,
                        signature: Some(format!(
                            "const {decl_name} = {}",
                            if container == ContainerKind::Union { "union" } else { "struct" }
                        )),
                        doc_comment: doc,
                        scope_path: None,
                        parent_index: None,
                    });
                    extract_struct_body(&body_lines, start_line, parent_idx, &mut symbols, &mut refs);
                    i = end_line as usize + 1;
                    continue;
                }
                ContainerKind::Enum | ContainerKind::Error => {
                    let (end_line, body_lines) = skip_block_collecting(&lines, i);
                    let parent_idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        name: decl_name.clone(),
                        qualified_name: decl_name.clone(),
                        kind: SymbolKind::Enum,
                        visibility: Some(if is_pub { Visibility::Public } else { Visibility::Private }),
                        start_line,
                        end_line,
                        start_col: 0,
                        end_col: 0,
                        signature: Some(format!(
                            "const {decl_name} = {}",
                            if container == ContainerKind::Error { "error" } else { "enum" }
                        )),
                        doc_comment: doc,
                        scope_path: None,
                        parent_index: None,
                    });
                    extract_enum_body(&body_lines, start_line, parent_idx, &mut symbols, &mut refs);
                    i = end_line as usize + 1;
                    continue;
                }
                ContainerKind::None => {
                    // Plain variable/constant
                    let decl_idx = symbols.len();
                    symbols.push(ExtractedSymbol {
                        name: decl_name.clone(),
                        qualified_name: decl_name.clone(),
                        kind: SymbolKind::Variable,
                        visibility: Some(if is_pub { Visibility::Public } else { Visibility::Private }),
                        start_line,
                        end_line: start_line,
                        start_col: 0,
                        end_col: 0,
                        signature: Some(trimmed.to_string()),
                        doc_comment: doc,
                        scope_path: None,
                        parent_index: None,
                    });
                    // Scan the declaration line for @builtin( calls
                    // (e.g. `const X = @This()`, `const X = @cImport({...})`,
                    //        `const X = @Vector(2, f32)`)
                    extract_builtin_calls_from_line(trimmed, decl_idx, start_line, &mut refs);
                }
            }
        }

        i += 1;
    }

    crate::types::ExtractionResult::new(symbols, refs, false)
}

// ---------------------------------------------------------------------------
// Struct body — extract fields and nested fn declarations
// ---------------------------------------------------------------------------

fn extract_struct_body(
    body_lines: &[(u32, &str)],
    _base_line: u32,
    parent_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut j = 0;
    while j < body_lines.len() {
        let (line_num, raw) = body_lines[j];
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed.starts_with("//") {
            j += 1;
            continue;
        }

        // Nested fn → Method
        if let Some((fn_name, is_pub, signature)) = parse_fn_declaration(trimmed) {
            // Find the extent of this nested fn within body_lines
            let start_j = j;
            let mut depth = 0i32;
            let mut end_j = j;
            // Count braces from this line forward
            for (k, (_, bl)) in body_lines[start_j..].iter().enumerate() {
                for ch in bl.chars() {
                    if ch == '{' { depth += 1; }
                    else if ch == '}' {
                        depth -= 1;
                        if depth <= 0 {
                            end_j = start_j + k;
                            break;
                        }
                    }
                }
                if depth <= 0 { break; }
            }

            let fn_idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: fn_name.clone(),
                qualified_name: fn_name.clone(),
                kind: SymbolKind::Method,
                visibility: Some(if is_pub { Visibility::Public } else { Visibility::Private }),
                start_line: line_num,
                end_line: body_lines.get(end_j).map(|(l, _)| *l).unwrap_or(line_num),
                start_col: 0,
                end_col: 0,
                signature: Some(signature),
                doc_comment: None,
                scope_path: None,
                parent_index: Some(parent_idx),
            });

            // Scan body for calls
            if end_j > start_j {
                let fn_body = &body_lines[start_j + 1..end_j];
                extract_calls_from_body_slice(fn_body, fn_idx, refs);
            }

            j = end_j + 1;
            continue;
        }

        // Struct field: `name: Type` or `name: Type = default`
        if let Some((field_name, type_name)) = parse_struct_field(trimmed) {
            let field_idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: field_name.clone(),
                qualified_name: field_name.clone(),
                kind: SymbolKind::Field,
                visibility: Some(Visibility::Public),
                start_line: line_num,
                end_line: line_num,
                start_col: 0,
                end_col: 0,
                signature: Some(format!("{field_name}: {type_name}")),
                doc_comment: None,
                scope_path: None,
                parent_index: Some(parent_idx),
            });

            // Emit TypeRef for non-primitive types
            if !is_primitive(&type_name) && type_name.chars().next().map_or(false, |c| c.is_alphanumeric() || c == '_') {
                refs.push(ExtractedRef {
                    source_symbol_index: field_idx,
                    target_name: type_name,
                    kind: EdgeKind::TypeRef,
                    line: line_num,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        }

        j += 1;
    }
}

// ---------------------------------------------------------------------------
// Enum body — extract members
// ---------------------------------------------------------------------------

fn extract_enum_body(
    body_lines: &[(u32, &str)],
    _base_line: u32,
    parent_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut j = 0;
    while j < body_lines.len() {
        let (line_num, raw) = body_lines[j];
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            j += 1;
            continue;
        }

        // fn declarations inside enum bodies (Zig allows methods on enums)
        if let Some((fn_name, is_pub, signature)) = parse_fn_declaration(trimmed) {
            let mut depth = 0i32;
            let mut end_j = j;
            for (k, (_, bl)) in body_lines[j..].iter().enumerate() {
                for ch in bl.chars() {
                    if ch == '{' { depth += 1; }
                    else if ch == '}' {
                        depth -= 1;
                        if depth <= 0 {
                            end_j = j + k;
                            break;
                        }
                    }
                }
                if depth <= 0 { break; }
            }

            let fn_idx = symbols.len();
            symbols.push(ExtractedSymbol {
                name: fn_name.clone(),
                qualified_name: fn_name.clone(),
                kind: SymbolKind::Method,
                visibility: Some(if is_pub { Visibility::Public } else { Visibility::Private }),
                start_line: line_num,
                end_line: body_lines.get(end_j).map(|(l, _)| *l).unwrap_or(line_num),
                start_col: 0,
                end_col: 0,
                signature: Some(signature),
                doc_comment: None,
                scope_path: None,
                parent_index: Some(parent_idx),
            });

            if end_j > j {
                let fn_body = &body_lines[j + 1..end_j];
                extract_calls_from_body_slice(fn_body, fn_idx, refs);
            }

            j = end_j + 1;
            continue;
        }

        // Enum member: identifier possibly followed by `= value,` or `,`
        let member = trimmed
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .next()
            .unwrap_or("")
            .trim();
        if member.is_empty() || ZIG_KEYWORDS.contains(&member) {
            j += 1;
            continue;
        }
        symbols.push(ExtractedSymbol {
            name: member.to_string(),
            qualified_name: member.to_string(),
            kind: SymbolKind::EnumMember,
            visibility: Some(Visibility::Public),
            start_line: line_num,
            end_line: line_num,
            start_col: 0,
            end_col: 0,
            signature: Some(member.to_string()),
            doc_comment: None,
            scope_path: None,
            parent_index: Some(parent_idx),
        });
        j += 1;
    }
}

/// Scan a single source line for `@builtin(` patterns and emit Calls refs.
/// Used for top-level declaration lines where the RHS contains builtin calls
/// (e.g. `const X = @This()`, `const X = @cImport({...})`).
/// Skips `@import` which is already handled as an Imports ref.
fn extract_builtin_calls_from_line(
    line: &str,
    source_symbol_index: usize,
    line_num: u32,
    refs: &mut Vec<ExtractedRef>,
) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' && i + 1 < bytes.len() && is_ident_start(bytes[i + 1]) {
            let start = i + 1;
            i = start;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let ident = &line[start..i];
            // Skip @import (already handled) and emit refs for all other builtins
            if i < bytes.len() && bytes[i] == b'(' && ident != "import" {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: format!("@{ident}"),
                    kind: EdgeKind::Calls,
                    line: line_num,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        } else {
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Anonymous struct extraction from function bodies
// ---------------------------------------------------------------------------

/// Scan a function body for struct blocks that contain fn/method declarations.
/// This handles three Zig patterns:
///   1. `return struct { ... }` — comptime generic type factory
///   2. `=> struct { ... }` — switch arm returning a struct type
///   3. `const Name = struct { ... }` — local named struct (vtable impl, etc.)
fn extract_anon_struct_fns(
    body_lines: &[(u32, &str)],
    parent_fn_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut j = 0;
    while j < body_lines.len() {
        let (line_num, raw) = body_lines[j];
        let trimmed = raw.trim();

        if trimmed.is_empty() || trimmed.starts_with("//") {
            j += 1;
            continue;
        }

        // Check for any pattern that opens a struct block in this line.
        // We look for `struct {` appearing anywhere in the line.
        if trimmed.contains("struct {") || trimmed.contains("struct{") {
            // Find the opening `{` that starts the struct body.
            // We need to locate the struct's own `{` — scan for `struct {`
            // and track depth from that point.
            if let Some(struct_brace_pos) = find_struct_open_brace(trimmed) {
                let _ = struct_brace_pos; // we just need to know one exists

                // Collect the body of this struct block from body_lines[j..].
                let mut depth = 0i32;
                let mut end_j = j;
                let mut struct_body: Vec<(u32, &str)> = Vec::new();
                let mut in_struct = false;

                for (k, (ln, bl)) in body_lines[j..].iter().enumerate() {
                    for ch in bl.chars() {
                        if ch == '{' {
                            depth += 1;
                            if depth == 1 {
                                in_struct = true;
                            }
                        } else if ch == '}' {
                            depth -= 1;
                            if depth <= 0 {
                                end_j = j + k;
                                break;
                            }
                        }
                    }
                    if in_struct && k > 0 && depth > 0 {
                        struct_body.push((*ln, bl));
                    }
                    if depth <= 0 && in_struct { break; }
                }

                if !struct_body.is_empty() {
                    // Extract fn declarations from the struct body
                    extract_struct_body(&struct_body, line_num, parent_fn_idx, symbols, refs);
                    // Recurse for deeper nesting
                    extract_anon_struct_fns(&struct_body, parent_fn_idx, symbols, refs);
                }

                j = end_j + 1;
                continue;
            }
        }

        j += 1;
    }
}

/// Returns true if `trimmed` contains `struct {` as the opening of a struct
/// block (not inside a string or comment).
fn find_struct_open_brace(trimmed: &str) -> Option<usize> {
    // Simple heuristic: look for "struct {" or "struct{" preceded by space/=> or start
    if let Some(pos) = trimmed.find("struct {") {
        return Some(pos);
    }
    if let Some(pos) = trimmed.find("struct{") {
        return Some(pos);
    }
    None
}


// ---------------------------------------------------------------------------
// Call extraction from body lines
// ---------------------------------------------------------------------------

fn extract_calls_from_body(
    body_lines: &[(u32, &str)],
    source_symbol_index: usize,
    _base_line: u32,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_calls_from_body_slice(body_lines, source_symbol_index, refs);
}

fn extract_calls_from_body_slice(
    body_lines: &[(u32, &str)],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    for (line_num, raw) in body_lines {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        // Find all `identifier(` patterns — these are function calls
        extract_call_identifiers(trimmed, source_symbol_index, *line_num, refs);
    }
}

/// Scan a line for `identifier(` and `@builtin(` call patterns.
///
/// Emits:
/// - `EdgeKind::Calls` for regular function calls (`ident(`)
/// - `EdgeKind::Calls` for Zig builtin function calls (`@ident(`) except
///   `@import` which is handled separately as `EdgeKind::Imports`.
fn extract_call_identifiers(
    line: &str,
    source_symbol_index: usize,
    line_num: u32,
    refs: &mut Vec<ExtractedRef>,
) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Builtin function call: `@identifier(`
        if bytes[i] == b'@' && i + 1 < bytes.len() && is_ident_start(bytes[i + 1]) {
            let start = i + 1; // skip the `@`
            i = start;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            let ident = &line[start..i];
            // Skip @import — already emitted as Imports ref in parse_var_declaration
            if i < bytes.len() && bytes[i] == b'(' && ident != "import" {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: format!("@{ident}"),
                    kind: EdgeKind::Calls,
                    line: line_num,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
            continue;
        }

        // Regular identifier call: `identifier(`
        if !is_ident_start(bytes[i]) {
            i += 1;
            continue;
        }
        // Collect the identifier
        let start = i;
        while i < bytes.len() && is_ident_char(bytes[i]) {
            i += 1;
        }
        let ident = &line[start..i];

        // Check if followed immediately by `(`
        if i < bytes.len() && bytes[i] == b'(' {
            if !ZIG_KEYWORDS.contains(&ident) && !is_primitive(ident) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: ident.to_string(),
                    kind: EdgeKind::Calls,
                    line: line_num,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                });
            }
        }
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_primitive(name: &str) -> bool {
    PRIMITIVES.contains(&name)
}

// ---------------------------------------------------------------------------
// Line parsers
// ---------------------------------------------------------------------------

/// Parse `test "name" {` or `test name {` — returns the test name.
fn parse_test_declaration(line: &str) -> Option<String> {
    let rest = line.strip_prefix("test ")?;
    let rest = rest.trim_start();
    // Quoted string: test "my test" {
    if let Some(inner) = rest.strip_prefix('"') {
        let end = inner.find('"')?;
        return Some(inner[..end].to_string());
    }
    // Identifier: test myTest {
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || ZIG_KEYWORDS.contains(&name.as_str()) {
        return None;
    }
    Some(name)
}

/// Parse function declaration lines. Returns (name, is_pub, signature).
/// Handles: `pub fn name(...)`, `fn name(...)`, `pub export fn name(...)`, etc.
fn parse_fn_declaration(line: &str) -> Option<(String, bool, String)> {
    let mut rest = line;
    let mut is_pub = false;

    if let Some(r) = rest.strip_prefix("pub ") {
        is_pub = true;
        rest = r.trim_start();
    }

    // Skip export/extern/inline/noinline modifiers
    for prefix in &["export ", "extern ", "inline ", "noinline "] {
        if let Some(r) = rest.strip_prefix(prefix) {
            rest = r.trim_start();
        }
    }

    let rest = rest.strip_prefix("fn ")?;
    let rest = rest.trim_start();

    // Extract name — up to '(' or whitespace
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();

    if name.is_empty() {
        return None;
    }

    // Build a terse signature: everything up to the first '{', ';', or 100 chars
    let sig_end = line
        .find('{')
        .or_else(|| line.find(';'))
        .unwrap_or(line.len().min(120));
    let signature = line[..sig_end].trim().to_string();

    Some((name, is_pub, signature))
}

/// Parse `const/var name = ...` declarations.
/// Returns (name, is_pub, container_kind, import_path).
fn parse_var_declaration(line: &str) -> Option<(String, bool, ContainerKind, Option<String>)> {
    let mut rest = line;
    let mut is_pub = false;

    if let Some(r) = rest.strip_prefix("pub ") {
        is_pub = true;
        rest = r.trim_start();
    }

    // Must start with const or var
    let rest = if let Some(r) = rest.strip_prefix("const ") {
        r.trim_start()
    } else if let Some(r) = rest.strip_prefix("var ") {
        r.trim_start()
    } else {
        return None;
    };

    // Name — up to ':' or '=' or whitespace
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return None;
    }

    // Skip to the '=' sign
    let eq_pos = line.find('=')?;
    let rhs = line[eq_pos + 1..].trim();

    // @import case
    if rhs.starts_with("@import(") {
        let inner = rhs.strip_prefix("@import(")?;
        let inner = inner.trim_start_matches('"');
        let path_end = inner.find('"').unwrap_or(inner.len());
        let path = inner[..path_end].to_string();
        return Some((name, is_pub, ContainerKind::None, Some(path)));
    }

    // Container types
    let container = if rhs.starts_with("struct ") || rhs.starts_with("struct{") || rhs == "struct{}" {
        ContainerKind::Struct
    } else if rhs.starts_with("packed struct") || rhs.starts_with("extern struct") {
        ContainerKind::Struct
    } else if rhs.starts_with("enum(") || rhs.starts_with("enum {") || rhs.starts_with("enum{") {
        ContainerKind::Enum
    } else if rhs.starts_with("union(") || rhs.starts_with("union {") || rhs.starts_with("union{") {
        ContainerKind::Union
    } else if rhs.starts_with("error {") || rhs.starts_with("error{") {
        ContainerKind::Error
    } else {
        ContainerKind::None
    };

    Some((name, is_pub, container, None))
}

/// Parse a struct field line: `name: Type` or `name: Type = default,`
fn parse_struct_field(line: &str) -> Option<(String, String)> {
    // Skip @attributes and keywords
    if line.starts_with('@') || line.starts_with("pub ") || line.starts_with("fn ") {
        return None;
    }
    let colon_pos = line.find(':')?;
    let name = line[..colon_pos].trim();
    if name.is_empty() || name.contains(' ') || ZIG_KEYWORDS.contains(&name) {
        return None;
    }
    let type_rest = line[colon_pos + 1..].trim();
    // Type ends at `=`, `,`, or end of line
    let type_end = type_rest
        .find(|c: char| c == '=' || c == ',')
        .unwrap_or(type_rest.len());
    let type_name = type_rest[..type_end].trim().to_string();
    if type_name.is_empty() {
        return None;
    }
    Some((name.to_string(), type_name))
}

// ---------------------------------------------------------------------------
// Block skipping
// ---------------------------------------------------------------------------

/// Skip from the line containing an opening `{` to the matching `}`.
/// Returns (end_line_index, body_lines_as_owned_strings).
///
/// Handles single-line `fn foo() void {}` by returning (i, []).
fn skip_block(lines: &[&str], start: usize) -> (u32, Vec<(u32, String)>) {
    let (end, body) = skip_block_collecting_owned(lines, start);
    (end, body)
}

fn skip_block_collecting<'a>(lines: &'a [&'a str], start: usize) -> (u32, Vec<(u32, &'a str)>) {
    let mut depth = 0i32;
    let mut body: Vec<(u32, &str)> = Vec::new();
    let mut end = start as u32;

    for (k, &line) in lines[start..].iter().enumerate() {
        let abs = start + k;
        for ch in line.chars() {
            if ch == '{' { depth += 1; }
            else if ch == '}' {
                depth -= 1;
                if depth <= 0 {
                    end = abs as u32;
                    return (end, body);
                }
            }
        }
        if abs > start && depth > 0 {
            body.push((abs as u32, line));
        }
    }
    (end, body)
}

fn skip_block_collecting_owned(lines: &[&str], start: usize) -> (u32, Vec<(u32, String)>) {
    let mut depth = 0i32;
    let mut body: Vec<(u32, String)> = Vec::new();
    let mut end = start as u32;

    for (k, &line) in lines[start..].iter().enumerate() {
        let abs = start + k;
        for ch in line.chars() {
            if ch == '{' { depth += 1; }
            else if ch == '}' {
                depth -= 1;
                if depth <= 0 {
                    end = abs as u32;
                    return (end, body);
                }
            }
        }
        if abs > start && depth > 0 {
            body.push((abs as u32, line.to_string()));
        }
    }
    (end, body)
}

// ---------------------------------------------------------------------------
// Doc comment collection
// ---------------------------------------------------------------------------

fn collect_doc_comments(lines: &[&str], block_line: usize) -> Option<String> {
    if block_line == 0 {
        return None;
    }
    let mut doc_lines: Vec<&str> = Vec::new();
    let mut j = block_line as isize - 1;
    while j >= 0 {
        let l = lines[j as usize].trim();
        if let Some(stripped) = l.strip_prefix("///") {
            doc_lines.push(stripped.trim());
            j -= 1;
        } else {
            break;
        }
    }
    if doc_lines.is_empty() {
        return None;
    }
    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}
