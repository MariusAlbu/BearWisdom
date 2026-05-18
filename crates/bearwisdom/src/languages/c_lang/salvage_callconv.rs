// =============================================================================
// c_lang/salvage_callconv.rs  —  recover MSVC SAL / __cdecl / WINAPI decls
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};

use super::salvage_text::{collect_balanced_parens, is_ident_byte};

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
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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
