// =============================================================================
// c_lang/salvage_macro_expand.rs  —  generic macro-catalog-driven declaration
// recovery (typedef / struct / function decls hidden behind project macros)
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};

use super::salvage_text::{collect_balanced_parens, is_ident_byte};

/// Generic macro-expansion salvage. For every macro defined in the project's
/// neighbouring headers and invoked in `source`, substitute the call's
/// arguments into the macro body and re-run the declaration scanners that
/// already handled the original source text. Symbols extracted from the
/// expansion get added to `symbols` if they aren't already present.
///
/// This is the architectural alternative to hand-maintaining specific
/// macro families like `CLAY__ARRAY_DEFINE` or `__DEFINE_CPP_OVERLOAD`.
/// Every macro the project itself defines becomes inputs to the expansion;
/// no name is hardcoded.
pub(super) fn salvage_macro_expanded_decls(
    source: &str,
    file_path: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let catalog = super::macro_catalog::catalog_for_file(file_path);
    if catalog.is_empty() { return }

    let invocations = scan_known_macro_invocations(source, &catalog);
    if invocations.is_empty() { return }

    let mut existing: std::collections::HashSet<String> =
        symbols.iter().map(|s| s.name.clone()).collect();

    for inv in invocations {
        let Some(def) = catalog.by_name.get(&inv.name) else { continue };
        let arg_strs: Vec<&str> = inv.args.iter().map(|s| s.as_str()).collect();
        let expanded = super::macro_catalog::expand(def, &arg_strs);
        if expanded.trim().is_empty() { continue }
        let fully_expanded = expand_recursively(&expanded, &catalog, 4);
        extract_decls_from_expansion(&fully_expanded, inv.line, symbols, &mut existing);
    }
}

/// Iteratively expand nested macro invocations in `text` until no more
/// catalog entries match (or `max_depth` is reached). Many real macros
/// chain — e.g. `CLAY__ARRAY_DEFINE` expands to a nested call to
/// `CLAY__ARRAY_DEFINE_FUNCTIONS`. Without the second pass the inner
/// declarations stay opaque.
fn expand_recursively(
    text: &str,
    catalog: &super::macro_catalog::MacroCatalog,
    max_depth: usize,
) -> String {
    let mut current = text.to_string();
    for _ in 0..max_depth {
        let next = expand_one_pass(&current, catalog);
        if next == current { return current }
        current = next;
    }
    current
}

fn expand_one_pass(
    text: &str,
    catalog: &super::macro_catalog::MacroCatalog,
) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if !(b.is_ascii_alphabetic() || b == b'_')
            || (i > 0 && is_ident_byte(bytes[i - 1]))
        {
            out.push(b as char);
            i += 1;
            continue;
        }
        let name_start = i;
        let mut j = i + 1;
        while j < bytes.len() && is_ident_byte(bytes[j]) { j += 1; }
        let name = &text[name_start..j];

        let Some(def) = catalog.by_name.get(name) else {
            out.push_str(name);
            i = j;
            continue;
        };
        if def.args.is_empty() {
            out.push_str(name);
            i = j;
            continue;
        }
        // Need `(` immediately (with optional whitespace) to be an
        // invocation.
        let mut k = j;
        while k < bytes.len() && bytes[k].is_ascii_whitespace() { k += 1; }
        if bytes.get(k).copied() != Some(b'(') {
            out.push_str(name);
            i = j;
            continue;
        }
        let Some((args_str, end)) = collect_balanced_parens(text, k) else {
            out.push_str(name);
            i = j;
            continue;
        };
        let args = split_macro_args(&args_str);
        if args.len() != def.args.len() {
            out.push_str(&text[name_start..end]);
            i = end;
            continue;
        }
        let arg_refs: Vec<&str> = args.iter().map(|a| a.as_str()).collect();
        let expanded = super::macro_catalog::expand(def, &arg_refs);
        out.push_str(&expanded);
        i = end;
    }
    out
}

struct KnownMacroInvocation {
    name: String,
    args: Vec<String>,
    line: usize,
}

fn scan_known_macro_invocations(
    source: &str,
    catalog: &super::macro_catalog::MacroCatalog,
) -> Vec<KnownMacroInvocation> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if !(b.is_ascii_alphabetic() || b == b'_') {
            i += 1;
            continue;
        }
        // Identifiers preceded by another ident char are continuations of
        // a longer name — already handled by an earlier iteration.
        if i > 0 && is_ident_byte(bytes[i - 1]) {
            i += 1;
            continue;
        }
        let name_start = i;
        i += 1;
        while i < bytes.len() && is_ident_byte(bytes[i]) {
            i += 1;
        }
        let name = &source[name_start..i];
        // Skip identifiers that aren't part of a function-like macro
        // catalog entry. Object-like macros don't expand to declarations
        // in the call-site shape we look for here.
        let Some(def) = catalog.by_name.get(name) else { continue };
        if def.args.is_empty() { continue }

        // Macro must be followed by `(` on the same line.
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            if bytes[j] == b'\n' { break }
            j += 1;
        }
        if bytes.get(j).copied() != Some(b'(') { continue }
        let Some((args_str, end)) = collect_balanced_parens(source, j) else { continue };
        let args = split_macro_args(&args_str);
        if args.len() != def.args.len() { continue }

        out.push(KnownMacroInvocation {
            name: name.to_string(),
            args,
            line,
        });
        line += source[j..end].bytes().filter(|c| *c == b'\n').count();
        i = end;
    }
    out
}

fn split_macro_args(args: &str) -> Vec<String> {
    let bytes = args.as_bytes();
    let mut out = Vec::new();
    let mut depth_paren = 0usize;
    let mut depth_angle = 0usize;
    let mut start = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth_paren += 1,
            b')' => depth_paren = depth_paren.saturating_sub(1),
            b'<' => depth_angle += 1,
            b'>' => depth_angle = depth_angle.saturating_sub(1),
            b',' if depth_paren == 0 && depth_angle == 0 => {
                out.push(args[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(args[start..].trim().to_string());
    out
}

/// Run the declaration scanners that ran on the original source over the
/// expanded macro body. Splits on top-level `;` (declaration end) and `}`
/// (function-body end) so multi-statement function definitions inside a
/// macro body don't get chopped on the `;` characters of their own
/// internal statements.
fn extract_decls_from_expansion(
    expanded: &str,
    line_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    existing: &mut std::collections::HashSet<String>,
) {
    for stmt in split_top_level(expanded) {
        let trimmed = stmt.trim();
        if trimmed.is_empty() { continue }
        // A function definition's `{...}` body is included in `trimmed`.
        // We only need the prefix up to the first `{` — that's the
        // signature where the name lives.
        let header = match trimmed.find('{') {
            Some(idx) => &trimmed[..idx],
            None => trimmed,
        };
        let header = header.trim();
        if header.is_empty() { continue }
        // Typedef alias names live AFTER the `{...}` block — pass the full
        // statement so the helper sees the `} NAME;` tail.
        try_emit_typedef(trimmed, line_idx, symbols, existing);
        try_emit_struct_def(trimmed, line_idx, symbols, existing);
        // Function declarations have the name BEFORE the `(`, which is in
        // the header. Passing the full stmt would let `try_emit_function_decl`
        // pick up the function-pointer parameter inside the body, so keep
        // the header restriction here.
        try_emit_function_decl(header, line_idx, symbols, existing);
    }
}

/// Split text on `;` (depth 0) and on `}` (depth 0) only when the
/// matching `{` opened a function body — i.e. the immediately preceding
/// non-whitespace character was `)`. Type-defining `{...}` blocks
/// (`struct { ... } NAME;`) keep their closing `}` in the same slice as
/// the trailing alias name and terminating `;`, so `try_emit_typedef`
/// sees the full declaration.
fn split_top_level(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    // Stack of (open_idx, is_function_body) for each open `{` we've seen.
    let mut brace_stack: Vec<bool> = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                let is_func = preceding_non_ws_byte(bytes, i) == Some(b')');
                brace_stack.push(is_func);
            }
            b'}' => {
                if let Some(is_func) = brace_stack.pop() {
                    if brace_stack.is_empty() && is_func {
                        out.push(&text[start..=i]);
                        start = i + 1;
                    }
                }
            }
            b';' if brace_stack.is_empty() => {
                out.push(&text[start..=i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < bytes.len() {
        out.push(&text[start..]);
    }
    out
}

fn preceding_non_ws_byte(bytes: &[u8], idx: usize) -> Option<u8> {
    let mut p = idx;
    while p > 0 {
        p -= 1;
        if !bytes[p].is_ascii_whitespace() {
            return Some(bytes[p]);
        }
    }
    None
}

/// `typedef <body> NAME;` — pluck NAME (last identifier before the
/// trailing `;`, ignoring any internal `;` that appear inside `{...}`
/// member-list bodies).
fn try_emit_typedef(
    stmt: &str,
    line_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    existing: &mut std::collections::HashSet<String>,
) {
    let Some(rest) = stmt.strip_prefix("typedef") else { return };
    let rest = rest.trim_start().trim_end();
    if rest.is_empty() { return }
    let bytes = rest.as_bytes();
    let mut end = bytes.len();
    // Strip a single trailing `;`.
    if end > 0 && bytes[end - 1] == b';' { end -= 1; }
    while end > 0 && bytes[end - 1].is_ascii_whitespace() { end -= 1; }
    // Strip trailing `[...]` array suffixes.
    while end > 0 && bytes[end - 1] == b']' {
        if let Some(open) = rest[..end].rfind('[') {
            end = open;
            while end > 0 && bytes[end - 1].is_ascii_whitespace() { end -= 1; }
        } else { break }
    }
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) { start -= 1; }
    if start == end { return }
    let name = &rest[start..end];
    // Filter the `struct`/`union`/`enum` keyword which can appear at the
    // very end of a forward typedef like `typedef struct Foo;` (rare).
    if matches!(name, "struct" | "union" | "enum") { return }
    push_if_missing(symbols, existing, name, SymbolKind::TypeAlias, line_idx,
        Some(format!("typedef ... {name}")));
}

/// `struct NAME { ... }` / `union NAME { ... }` / `enum NAME { ... }`.
/// Emits a Struct symbol for the named tag.
fn try_emit_struct_def(
    stmt: &str,
    line_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    existing: &mut std::collections::HashSet<String>,
) {
    for keyword in ["struct ", "union ", "enum "] {
        if let Some(rest) = stmt.find(keyword) {
            let after = &stmt[rest + keyword.len()..];
            let after = after.trim_start();
            let bytes = after.as_bytes();
            let mut k = 0;
            while k < bytes.len() && is_ident_byte(bytes[k]) { k += 1; }
            if k == 0 { continue }
            let name = &after[..k];
            // Require a `{` somewhere after the name (definition, not ref).
            let after_name = &after[k..];
            if !after_name.contains('{') { continue }
            push_if_missing(symbols, existing, name, SymbolKind::Struct, line_idx,
                Some(format!("{}{}", keyword, name)));
            return;
        }
    }
}

/// `<return-type> NAME(<params>)` — match a function-decl shape and emit
/// NAME. Permits optional `*` between return type and name (`Foo *bar(...)`).
fn try_emit_function_decl(
    stmt: &str,
    line_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    existing: &mut std::collections::HashSet<String>,
) {
    let bytes = stmt.as_bytes();
    // Find the rightmost identifier immediately before a `(`.
    let Some(open_paren) = stmt.find('(') else { return };
    if open_paren == 0 { return }
    let mut name_end = open_paren;
    while name_end > 0 && bytes[name_end - 1].is_ascii_whitespace() { name_end -= 1; }
    if name_end == 0 { return }
    let mut name_start = name_end;
    while name_start > 0 && is_ident_byte(bytes[name_start - 1]) { name_start -= 1; }
    if name_start == name_end { return }
    // Function-pointer-style decls have `(*NAME)(` — a `(` immediately
    // before the `*`. `<type> *NAME(` is a normal function returning a
    // pointer and must NOT be rejected. Walk back over the prefix:
    //   `(` then `*` (with optional whitespace between) → function ptr,
    //   anything else → ordinary function decl.
    if name_start > 0 {
        let mut p = name_start;
        while p > 0 && bytes[p - 1].is_ascii_whitespace() { p -= 1; }
        if p > 0 && bytes[p - 1] == b'*' {
            // Could be `(*NAME` or `<type> *NAME`. Walk past the `*` and
            // any whitespace; reject only when a `(` is at the next
            // significant byte to the left.
            p -= 1;
            while p > 0 && bytes[p - 1].is_ascii_whitespace() { p -= 1; }
            if p > 0 && bytes[p - 1] == b'(' {
                return;
            }
        }
    }
    let name = &stmt[name_start..name_end];
    // Filter common keywords that look like names but aren't.
    if matches!(name, "if" | "for" | "while" | "switch" | "return" | "sizeof" | "alignof") {
        return;
    }
    // Require a return-type token before the name (rules out bare calls
    // like `foo();` that aren't declarations). Cheapest check: at least one
    // ident character before name_start, separated by whitespace.
    let mut t = name_start;
    while t > 0 && bytes[t - 1].is_ascii_whitespace() { t -= 1; }
    if t == 0 { return }
    if !is_ident_byte(bytes[t - 1]) && bytes[t - 1] != b'*' { return }
    push_if_missing(symbols, existing, name, SymbolKind::Function, line_idx,
        Some(format!("{name}(...)")));
}

fn push_if_missing(
    symbols: &mut Vec<ExtractedSymbol>,
    existing: &mut std::collections::HashSet<String>,
    name: &str,
    kind: SymbolKind,
    line_idx: usize,
    signature: Option<String>,
) {
    if existing.contains(name) { return }
    let line_no = line_idx as u32;
    symbols.push(ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: None,
        start_line: line_no,
        end_line: line_no,
        start_col: 0,
        end_col: name.len() as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
    });
    existing.insert(name.to_string());
}
