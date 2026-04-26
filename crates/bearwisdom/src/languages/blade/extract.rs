//! Blade host-level extraction.
//!
//! The host produces ONE file-level `Class` symbol named after the
//! template path (so `resources/views/users/index.blade.php` becomes
//! `users.index`), plus one symbol or ref per directive that names a
//! section, component, slot, stack, included template, etc.

use std::path::Path;

use super::directives::{DEFINING_DIRECTIVES, REFERENCING_DIRECTIVES};
use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    // File-level symbol — its qualified_name is the dotted template
    // identifier so that `@include('users.show')` matches it later via
    // the resolver's name lookup.
    let template_name = template_name_from_path(file_path);
    let host_index = 0usize;
    symbols.push(ExtractedSymbol {
        name: template_name.clone(),
        qualified_name: template_name.clone(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    });

    // Directive scan — single pass over the source. Position for emitted
    // symbols and refs is the line where the `@directive(` token starts.
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            if let Some((name, after_name)) = read_directive_name(bytes, i + 1) {
                if let Some(payload_start) = first_paren_after(bytes, after_name) {
                    if let Some((arg, payload_end)) = read_first_string_arg(bytes, payload_start) {
                        let line = line_at(bytes, i);
                        if let Some(kind) = directive_symbol_kind(&name) {
                            let qname = format!("{template_name}.{arg}");
                            symbols.push(ExtractedSymbol {
                                name: arg.clone(),
                                qualified_name: qname,
                                kind,
                                visibility: Some(Visibility::Public),
                                start_line: line,
                                end_line: line,
                                start_col: 0,
                                end_col: 0,
                                signature: None,
                                doc_comment: None,
                                scope_path: Some(template_name.clone()),
                                parent_index: Some(host_index),
                            });
                            i = payload_end;
                            continue;
                        }
                        if REFERENCING_DIRECTIVES.contains(&name.as_str()) {
                            refs.push(ExtractedRef {
                                source_symbol_index: host_index,
                                target_name: arg,
                                kind: EdgeKind::Imports,
                                line,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
});
                            i = payload_end;
                            continue;
                        }
                    }
                }
            }
        }
        i += 1;
    }

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

fn directive_symbol_kind(name: &str) -> Option<SymbolKind> {
    DEFINING_DIRECTIVES
        .iter()
        .find_map(|(n, k)| if *n == name { Some(*k) } else { None })
}

fn read_directive_name(bytes: &[u8], start: usize) -> Option<(String, usize)> {
    let mut end = start;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_alphanumeric() || b == b'_' {
            end += 1;
        } else {
            break;
        }
    }
    if end == start { return None; }
    let s = std::str::from_utf8(&bytes[start..end]).ok()?;
    Some((s.to_string(), end))
}

fn first_paren_after(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') { i += 1; }
    if bytes.get(i) == Some(&b'(') { Some(i + 1) } else { None }
}

/// Read the first string-literal argument (single or double quoted) of
/// a directive call. Returns `(arg, byte_after_closing_paren)`. Returns
/// `None` when the first argument isn't a string literal — those
/// directives don't yield a graph-queryable symbol/ref.
fn read_first_string_arg(bytes: &[u8], paren_start: usize) -> Option<(String, usize)> {
    let mut i = paren_start;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') { i += 1; }
    let quote = match bytes.get(i) {
        Some(b'"') => b'"',
        Some(b'\'') => b'\'',
        _ => return None,
    };
    let arg_start = i + 1;
    let mut j = arg_start;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b if b == quote => break,
            _ => j += 1,
        }
    }
    if j >= bytes.len() { return None; }
    let arg = std::str::from_utf8(&bytes[arg_start..j]).ok()?.to_string();
    // Skip past close-quote and optional comma + remaining args; find matching `)`.
    let mut depth: i32 = 1;
    let mut k = j + 1;
    while k < bytes.len() && depth > 0 {
        match bytes[k] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'"' | b'\'' => k = skip_str_arg(bytes, k, bytes[k]) - 1,
            _ => {}
        }
        k += 1;
    }
    Some((arg, k))
}

fn skip_str_arg(bytes: &[u8], pos: usize, quote: u8) -> usize {
    let mut i = pos + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn line_at(bytes: &[u8], pos: usize) -> u32 {
    let mut line: u32 = 0;
    for b in bytes.iter().take(pos) {
        if *b == b'\n' { line += 1; }
    }
    line
}

/// Convert a Blade file path into Laravel's dotted template identifier:
/// `resources/views/users/show.blade.php` → `users.show`.
/// Falls back to the file stem (minus `.blade`) for paths that don't
/// contain a `views/` segment.
pub fn template_name_from_path(file_path: &str) -> String {
    let normalized = file_path.replace('\\', "/");
    let stem = match normalized.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/{}", strip_blade_ext(name)),
        None => strip_blade_ext(&normalized).to_string(),
    };
    let after = if let Some((_, rest)) = stem.rsplit_once("/views/") {
        rest.to_string()
    } else if let Some(rest) = stem.strip_prefix("views/") {
        rest.to_string()
    } else {
        stem.clone()
    };
    after.replace('/', ".")
}

fn strip_blade_ext(name: &str) -> &str {
    name.strip_suffix(".blade.php")
        .or_else(|| name.strip_suffix(".blade"))
        .or_else(|| Path::new(name).file_stem().and_then(|s| s.to_str()))
        .unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_name_from_views_path() {
        assert_eq!(
            template_name_from_path("resources/views/users/show.blade.php"),
            "users.show"
        );
        assert_eq!(
            template_name_from_path("resources/views/layouts/app.blade.php"),
            "layouts.app"
        );
    }

    #[test]
    fn template_name_without_views_dir_uses_stem() {
        assert_eq!(template_name_from_path("show.blade.php"), "show");
    }

    #[test]
    fn extends_emits_imports_ref() {
        let src = "@extends('layouts.app')\n<h1>x</h1>";
        let r = extract(src, "resources/views/users/show.blade.php");
        assert_eq!(r.refs.len(), 1);
        assert_eq!(r.refs[0].target_name, "layouts.app");
        assert_eq!(r.refs[0].kind, EdgeKind::Imports);
    }

    #[test]
    fn section_emits_symbol_under_template() {
        let src = "@extends('base')\n@section('content')\n<h1>hi</h1>\n@endsection";
        let r = extract(src, "resources/views/users/show.blade.php");
        let section = r.symbols.iter().find(|s| s.name == "content")
            .expect("content section symbol missing");
        assert_eq!(section.kind, SymbolKind::Method);
        assert_eq!(section.qualified_name, "users.show.content");
        assert_eq!(section.scope_path.as_deref(), Some("users.show"));
    }

    #[test]
    fn include_emits_imports_ref() {
        let src = "@include('partials.header')\n@include('partials.footer')\n";
        let r = extract(src, "resources/views/layouts/app.blade.php");
        let targets: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Imports)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(targets.contains(&"partials.header"));
        assert!(targets.contains(&"partials.footer"));
    }

    #[test]
    fn host_file_symbol_is_emitted_first() {
        let src = "<h1>plain</h1>";
        let r = extract(src, "resources/views/landing.blade.php");
        assert_eq!(r.symbols.len(), 1);
        assert_eq!(r.symbols[0].qualified_name, "landing");
        assert_eq!(r.symbols[0].kind, SymbolKind::Class);
    }

    #[test]
    fn double_quoted_strings_supported() {
        let src = "@extends(\"layouts.app\")";
        let r = extract(src, "x.blade.php");
        assert_eq!(r.refs[0].target_name, "layouts.app");
    }

    #[test]
    fn directives_with_non_string_arg_skipped() {
        // `@if($flag)` doesn't take a template name and shouldn't fire.
        let src = "@if($flag) hi @endif";
        let r = extract(src, "x.blade.php");
        assert_eq!(r.symbols.len(), 1, "only host symbol expected");
        assert!(r.refs.is_empty());
    }
}
