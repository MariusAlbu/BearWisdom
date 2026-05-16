// =============================================================================
// languages/c_lang/salvage.rs  —  raw-text fallbacks for tree-sitter recovery
// =============================================================================
//
// When tree-sitter-c / tree-sitter-cpp commit to error-recovery (typical
// triggers: unknown SDK macros like `REDISMODULE_API`, MSVC SAL annotations,
// `__declspec(dllexport)`, `_EXPORT_STD`, project-defined `*_API` markers),
// real declarations downstream of the recovery point either drop entirely
// or get misparsed. These line-scanning fallbacks recover the lost symbols
// by pattern-matching the source text, deduplicating against whatever the
// grammar did extract.

use std::collections::HashSet;

use crate::types::{ExtractedSymbol, SymbolKind};

// ---------------------------------------------------------------------------
// Salvage fallbacks
// ---------------------------------------------------------------------------

/// Scan source for `#define IDENT [...] ` lines and emit symbols for any
/// name not already in the symbols table. Function-like macros (`#define
/// FOO(a, b) ...`) emit as Function; object-like (`#define FOO bar`) emit
/// as Variable.
///
/// Runs unconditionally — the dedup against existing names makes it a
/// no-op when tree-sitter already extracted everything. Cost: one
/// lines() pass over the source, O(N) substring matching.
pub(super) fn salvage_missed_defines(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let mut existing: HashSet<String> =
        symbols.iter().map(|s| s.name.clone()).collect();

    for (line_idx, line) in source.lines().enumerate() {
        let stripped = line.trim_start();
        let after_hash = match stripped.strip_prefix('#') {
            Some(s) => s.trim_start(),
            None => continue,
        };
        let after_define = match after_hash.strip_prefix("define") {
            Some(s) => s,
            None => continue,
        };
        // Require whitespace after `define` so we don't accidentally
        // match `defined`, `defines`, etc.
        if !after_define.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let after_define = after_define.trim_start();
        // Parse the identifier.
        let mut iter = after_define.char_indices();
        let first = match iter.next() {
            Some((_, c)) if c.is_alphabetic() || c == '_' => c,
            _ => continue,
        };
        let mut end = first.len_utf8();
        for (idx, c) in iter {
            if c.is_alphanumeric() || c == '_' {
                end = idx + c.len_utf8();
            } else {
                break;
            }
        }
        let name = &after_define[..end];
        if existing.contains(name) {
            continue;
        }
        let is_function = after_define[end..].starts_with('(');
        let kind = if is_function {
            SymbolKind::Function
        } else {
            SymbolKind::Variable
        };
        let line_no = line_idx as u32;
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind,
            visibility: None,
            start_line: line_no,
            end_line: line_no,
            start_col: 0,
            end_col: end as u32,
            signature: Some(format!("#define {name}")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
        existing.insert(name.to_string());
    }
}

/// Scan source for function-pointer-table declarations of the shape
/// `<prefix> <return_type> (*NAME)(<params>) <suffix>;` and emit a Function
/// symbol for any NAME not already in the symbols table.
///
/// The `(*NAME)(` token sequence is unambiguous at file scope — it appears
/// only in function-pointer declarations and (rarely) inside parameter
/// lists. Restricting to lines that end in `;` filters most false hits.
/// Anything tree-sitter parsed correctly will already have a symbol with
/// the same name, so the dedup check makes us a no-op there.
pub(super) fn salvage_missed_function_pointer_decls(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    let mut existing: HashSet<String> =
        symbols.iter().map(|s| s.name.clone()).collect();

    for (line_idx, line) in source.lines().enumerate() {
        let trimmed = line.trim_end();
        // Require trailing `;` — declaration, not call site.
        if !trimmed.ends_with(';') {
            continue;
        }
        // Look for the `(*NAME)(` shape.
        let Some(name) = scan_funptr_decl_name(trimmed) else { continue };
        if existing.contains(name) {
            continue;
        }
        let line_no = line_idx as u32;
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Function,
            visibility: None,
            start_line: line_no,
            end_line: line_no,
            start_col: 0,
            end_col: name.len() as u32,
            signature: Some(trimmed.trim_start().to_string()),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
        existing.insert(name.to_string());
    }
}

/// Find `(*NAME)(` in `line` and return NAME, or None if the shape is absent.
fn scan_funptr_decl_name(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 4 < bytes.len() {
        // Match `(`, optional whitespace, `*`, optional whitespace, identifier, `)`, `(`.
        if bytes[i] != b'(' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
        if j >= bytes.len() || bytes[j] != b'*' { i += 1; continue; }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
        let name_start = j;
        while j < bytes.len()
            && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_')
        { j += 1; }
        let name_end = j;
        if name_end == name_start {
            i += 1;
            continue;
        }
        // First char must be alpha or `_`.
        let first = bytes[name_start];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            i += 1;
            continue;
        }
        while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
        if j >= bytes.len() || bytes[j] != b')' { i += 1; continue; }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() { j += 1; }
        if j >= bytes.len() || bytes[j] != b'(' { i += 1; continue; }
        return Some(&line[name_start..name_end]);
    }
    None
}

/// Scan source for MSVC stdlib function declarations of the shape
///   `<...> __cdecl NAME(`
/// and emit a Function symbol for any NAME not already in the symbols
/// table. Also matches `__CRTDECL`, `__stdcall`, `__fastcall`,
/// `__vectorcall`, and `WINAPI` / `APIENTRY` (which the SDK headers
/// macro-define to one of those calling conventions).
///
/// MSVC SDK headers (`stdio.h`, `string.h`, `windows.h`) declare CRT
/// functions with SAL annotations (`_Check_return_`, `_In_z_`) and
/// calling-convention attributes (`__cdecl`). Tree-sitter-cpp does not
/// preprocess macros, so unknown identifiers like `_Check_return_` push
/// the parser into recovery, after which the real function name ends
/// up nested inside what tree-sitter treats as the parameter list.
/// Symptom: 1k+ unresolved `printf` / `strlen` / `memcpy` refs on
/// every Windows project even with the SDK headers indexed.
///
/// The calling-convention token is unambiguous at file scope — it
/// appears only in function declarations. Matching `__cdecl NAME(` (or
/// equivalents) recovers the real symbol with no false positives.
pub(super) fn salvage_missed_msvc_calling_convention_decls(
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    use std::collections::HashSet;
    // Standard C / Windows-SDK calling convention keywords. These are
    // documented language extensions (MSVC `__cdecl` family, `WINAPI`/
    // `APIENTRY`/`CALLBACK` from `windef.h`). Project-specific decoration
    // macros (`ngx_cdecl`, `printflike`, ...) are *not* listed here —
    // those need to be discovered by parsing `#define` directives in the
    // project's own headers via a real preprocessor pass.
    const CONVENTIONS: &[&str] = &[
        "__cdecl",
        "__CRTDECL",
        "__stdcall",
        "__fastcall",
        "__vectorcall",
        "__thiscall",
        "WINAPI",
        "APIENTRY",
        "CALLBACK",
    ];

    let mut existing: HashSet<String> =
        symbols.iter().map(|s| s.name.clone()).collect();

    let lines: Vec<&str> = source.lines().collect();
    for (line_idx, line) in lines.iter().enumerate() {
        // Same-line shape: `<...> __cdecl NAME(`.
        if let Some(name) = scan_calling_convention_decl_name(line, CONVENTIONS) {
            if !existing.contains(name) {
                push_salvaged_function(symbols, &mut existing, name, line_idx);
            }
            continue;
        }
        // Multi-line Win32 shape, where the SDK headers split the
        // declaration across lines:
        //   WINBASEAPI
        //   VOID
        //   WINAPI
        //   EnterCriticalSection(
        //       _Inout_ LPCRITICAL_SECTION lpCriticalSection
        //       );
        // Recognize a bare `IDENT(` line whose previous non-empty
        // line is a calling-convention token on its own (or trailing
        // whitespace).
        if let Some(name) = scan_bare_funcname_paren(line) {
            if existing.contains(name) {
                continue;
            }
            // Walk back through whitespace-only lines.
            let mut j = line_idx;
            let prev_conv = loop {
                if j == 0 { break None; }
                j -= 1;
                let prev = lines[j].trim();
                if prev.is_empty() { continue; }
                break Some(prev);
            };
            let Some(prev) = prev_conv else { continue };
            if CONVENTIONS.iter().any(|c| prev == *c)
                || line_has_trailing_declaration_macro(prev, CONVENTIONS)
            {
                push_salvaged_function(symbols, &mut existing, name, line_idx);
            }
        }
    }
}

fn push_salvaged_function(
    symbols: &mut Vec<ExtractedSymbol>,
    existing: &mut std::collections::HashSet<String>,
    name: &str,
    line_idx: usize,
) {
    let line_no = line_idx as u32;
    symbols.push(ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: SymbolKind::Function,
        visibility: None,
        start_line: line_no,
        end_line: line_no,
        start_col: 0,
        end_col: name.len() as u32,
        signature: Some(format!("{name}(...)")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
    existing.insert(name.to_string());
}

/// Match a line of the shape `<whitespace?>IDENT(<...>` — used by the
/// multi-line calling-convention salvage to identify the function-name
/// line under a Win32-style multi-line declaration.
fn scan_bare_funcname_paren(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let start = i;
    if i >= bytes.len() || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return None;
    }
    while i < bytes.len() && is_ident_byte(bytes[i]) {
        i += 1;
    }
    let end = i;
    if end == start { return None }
    if i >= bytes.len() || bytes[i] != b'(' { return None }
    Some(&line[start..end])
}

fn line_has_trailing_declaration_macro(line: &str, conventions: &[&str]) -> bool {
    let bytes = line.as_bytes();
    for conv in conventions {
        let mut search_from = 0;
        while let Some(rel) = line[search_from..].find(conv) {
            let conv_start = search_from + rel;
            let conv_end = conv_start + conv.len();
            let before_ok = conv_start == 0
                || !is_ident_byte(bytes[conv_start - 1]);
            let after_byte = bytes.get(conv_end).copied();
            let after_ok = after_byte
                .map(|b| !is_ident_byte(b))
                .unwrap_or(true);
            if !before_ok || !after_ok {
                search_from = conv_end;
                continue;
            }

            let mut j = conv_end;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j).copied() == Some(b'(') {
                let Some((_args, end)) = collect_balanced_parens(line, j) else {
                    search_from = conv_end;
                    continue;
                };
                j = end;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
            }
            if j == bytes.len() {
                return true;
            }
            search_from = conv_end;
        }
    }
    false
}

/// Find `<convention> IDENT(` on `line` and return IDENT. Skips any
/// occurrence where the token after IDENT isn't `(` (e.g. typedef
/// usage `typedef int (__cdecl* CB)(...)` where the convention sits
/// before a `*`, not before an identifier).
fn scan_calling_convention_decl_name<'a>(
    line: &'a str,
    conventions: &[&str],
) -> Option<&'a str> {
    let bytes = line.as_bytes();
    for conv in conventions {
        let mut search_from = 0;
        while let Some(rel) = line[search_from..].find(conv) {
            let conv_start = search_from + rel;
            let conv_end = conv_start + conv.len();
            // The convention must be a whole token — bounded by
            // non-ident chars (or start/end of line).
            let before_ok = conv_start == 0
                || !is_ident_byte(bytes[conv_start - 1]);
            let after_byte = bytes.get(conv_end).copied();
            let after_ok = after_byte
                .map(|b| !is_ident_byte(b))
                .unwrap_or(false);
            if !before_ok || !after_ok {
                search_from = conv_end;
                continue;
            }
            // Skip whitespace and optional annotation macro arguments between
            // convention and name: `PRINTF_LIKE(4, 5) foo(...)` and
            // `printflike(1, 2) foo(...)` are project-local declaration
            // macros, not call sites.
            let mut j = conv_end;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j).copied() == Some(b'(') {
                if let Some((_args, end)) = collect_balanced_parens(line, j) {
                    j = end;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                }
            }
            // Pointer declarators immediately after a convention are function
            // pointer typedefs/variables (`__cdecl *foo`), not ordinary
            // function declarations. Return-type pointers appear before the
            // convention and are safe (`u_char * ngx_cdecl ngx_sprintf`).
            if j >= bytes.len() || bytes[j] == b'*' || bytes[j] == b'(' {
                search_from = conv_end;
                continue;
            }
            // Identifier.
            let name_start = j;
            if !(bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
                search_from = conv_end;
                continue;
            }
            while j < bytes.len() && is_ident_byte(bytes[j]) {
                j += 1;
            }
            let name_end = j;
            // Must be followed by `(`, with optional whitespace.
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= bytes.len() || bytes[j] != b'(' {
                search_from = conv_end;
                continue;
            }
            return Some(&line[name_start..name_end]);
        }
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn collect_balanced_parens(source: &str, open_idx: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    if bytes.get(open_idx).copied() != Some(b'(') {
        return None;
    }
    let mut depth = 1usize;
    let mut i = open_idx + 1;
    let args_start = i;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((source[args_start..i].to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

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
    });
    existing.insert(name.to_string());
}

/// Scan for `template <...> class NAME` and `template <...> struct
/// NAME` declarations and emit a Class/Struct symbol for any NAME
/// not already in the symbols table. Tolerates a balanced angle-
/// bracket parameter list and any prefix tokens between the
/// declaration and the start of the line — MSVC headers prepend
/// `_EXPORT_STD` (a C++20-modules export macro) and Boost-flavoured
/// libs use similar macro shims (`BOOST_SYMBOL_VISIBLE`, etc.).
///
/// Single line only — the `template <...>` opener and the class
/// keyword + name appear on one line in the cases we care about.
/// The MSVC `<memory>` shape is exactly `_EXPORT_STD template <class
/// _Ty>` followed by `class shared_ptr;` on the next line — see the
/// multi-line wrapper below.
pub(super) fn salvage_missed_template_class_decls(
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    use std::collections::HashSet;
    let mut existing: HashSet<String> =
        symbols.iter().map(|s| s.name.clone()).collect();

    let lines: Vec<&str> = source.lines().collect();
    for (line_idx, line) in lines.iter().enumerate() {
        // Single-line shape: `[prefix] template <...> class NAME` (with or
        // without trailing `;` / `{`).
        if let Some((name, kind)) = scan_template_class_decl(line) {
            if !existing.contains(name) {
                push_salvaged_class(symbols, &mut existing, name, kind, line_idx);
            }
            continue;
        }
        // Multi-line shape used by MSVC `<memory>`:
        //   _EXPORT_STD template <class _Ty>
        //   class shared_ptr;
        // The current line starts with `class NAME` or `struct NAME`,
        // and the previous non-empty line ended with `>` (closing the
        // template parameter list).
        if let Some((name, kind)) = scan_class_decl_only(line) {
            if existing.contains(name) {
                continue;
            }
            let mut j = line_idx;
            let prev_end_angle = loop {
                if j == 0 { break false; }
                j -= 1;
                let prev = lines[j].trim_end();
                if prev.is_empty() { continue; }
                break prev.ends_with('>');
            };
            if prev_end_angle {
                push_salvaged_class(symbols, &mut existing, name, kind, line_idx);
            }
        }
    }
}

fn push_salvaged_class(
    symbols: &mut Vec<ExtractedSymbol>,
    existing: &mut std::collections::HashSet<String>,
    name: &str,
    kind: SymbolKind,
    line_idx: usize,
) {
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
        signature: Some(format!(
            "{} {name}",
            if matches!(kind, SymbolKind::Struct) { "struct" } else { "class" }
        )),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });
    existing.insert(name.to_string());
}

/// Find `template <...> {class|struct} IDENT` on a line. Returns
/// `(name, kind)` if matched. `<...>` may be balanced angle brackets
/// nesting (`template<class T = std::pair<int, int>>`).
fn scan_template_class_decl(line: &str) -> Option<(&str, SymbolKind)> {
    let bytes = line.as_bytes();
    let template_pos = find_keyword(line, "template")?;
    let mut i = template_pos + "template".len();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'<' { return None }
    // Walk balanced angle brackets.
    let mut depth = 1usize;
    i += 1;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if depth != 0 { return None }
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let kind = if line[i..].starts_with("class") && bytes.get(i + 5).map(|b| !is_ident_byte(*b)).unwrap_or(true) {
        i += 5;
        SymbolKind::Class
    } else if line[i..].starts_with("struct") && bytes.get(i + 6).map(|b| !is_ident_byte(*b)).unwrap_or(true) {
        i += 6;
        SymbolKind::Struct
    } else {
        return None;
    };
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let name_start = i;
    if i >= bytes.len() || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return None;
    }
    while i < bytes.len() && is_ident_byte(bytes[i]) {
        i += 1;
    }
    let name_end = i;
    Some((&line[name_start..name_end], kind))
}

/// Find a bare `class IDENT` or `struct IDENT` at the start of a line
/// (allowing leading whitespace). Returns `(name, kind)` when matched.
fn scan_class_decl_only(line: &str) -> Option<(&str, SymbolKind)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let kind = if line[i..].starts_with("class") && bytes.get(i + 5).map(|b| !is_ident_byte(*b)).unwrap_or(true) {
        i += 5;
        SymbolKind::Class
    } else if line[i..].starts_with("struct") && bytes.get(i + 6).map(|b| !is_ident_byte(*b)).unwrap_or(true) {
        i += 6;
        SymbolKind::Struct
    } else {
        return None;
    };
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let name_start = i;
    if i >= bytes.len() || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return None;
    }
    while i < bytes.len() && is_ident_byte(bytes[i]) {
        i += 1;
    }
    let name_end = i;
    if name_end == name_start { return None }
    Some((&line[name_start..name_end], kind))
}

/// Find `keyword` as a whole token (bounded by non-ident chars) on
/// `line`, returning its byte offset.
fn find_keyword(line: &str, keyword: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = line[search_from..].find(keyword) {
        let start = search_from + rel;
        let end = start + keyword.len();
        let before_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_ident_byte(bytes[end]);
        if before_ok && after_ok {
            return Some(start);
        }
        search_from = end;
    }
    None
}
