// =============================================================================
// c_lang/salvage_template_class.rs  —  recover `template <...> class NAME`
// declarations behind export-macro shims (_EXPORT_STD, BOOST_SYMBOL_VISIBLE)
// =============================================================================

use crate::types::{ExtractedSymbol, SymbolKind};

use super::salvage_text::is_ident_byte;

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
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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
