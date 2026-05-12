//! Twig host-level extraction.
//!
//! Emits one file-level `Class` symbol named after the dotted template
//! path (Symfony / Drupal convention: `users/show.html.twig` →
//! `users.show`), one `Method` per `{% block name %}` and `{% macro
//! name(...) %}`, and one `Imports` ref per template-relating
//! directive (`extends`, `include`, `use`, `import`, `from`, `embed`).
//!
//! Twig expressions (`{{ expr }}`) are NOT routed through a sub-
//! extractor — there's no Twig expression grammar in the workspace and
//! Twig's filter / pipe syntax doesn't map onto PHP or Python without
//! significant adapter work. The plan is explicit about this trade-off.

use std::path::Path;

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

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

    // Scan for `{% tag ... %}` constructs.
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if has_prefix(bytes, i, b"{%") {
            // Skip optional `-` whitespace control marker.
            let body_start = if bytes.get(i + 2) == Some(&b'-') { i + 3 } else { i + 2 };
            // Find matching `%}` (also skip trailing `-`).
            let close = find_subseq(bytes, body_start, b"%}");
            if close.is_none() { break; }
            let close = close.unwrap();
            let body_end = if close > 0 && bytes[close - 1] == b'-' { close - 1 } else { close };
            if body_end > body_start {
                if let Some(body) = std::str::from_utf8(&bytes[body_start..body_end]).ok() {
                    handle_tag(
                        body.trim(),
                        line_at(bytes, i),
                        &template_name,
                        host_index,
                        &mut symbols,
                        &mut refs,
                    );
                }
            }
            i = close + 2;
            continue;
        }
        // Twig comment `{# ... #}` — skip to avoid matching things inside.
        if has_prefix(bytes, i, b"{#") {
            if let Some(end) = find_subseq(bytes, i + 2, b"#}") {
                i = end + 2;
                continue;
            }
            break;
        }
        i += 1;
    }

    ExtractionResult {
        symbols, refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: false,
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
    }
}

fn handle_tag(
    body: &str,
    line: u32,
    template_name: &str,
    host_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // First whitespace-separated token is the tag name.
    let mut parts = body.splitn(2, char::is_whitespace);
    let tag = parts.next().unwrap_or("").trim();
    let rest = parts.next().unwrap_or("").trim();
    match tag {
        "block" => {
            if let Some(name) = read_ident(rest) {
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: format!("{template_name}.{name}"),
                    kind: SymbolKind::Method,
                    visibility: Some(Visibility::Public),
                    start_line: line,
                    end_line: line,
                    start_col: 0,
                    end_col: 0,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some(template_name.to_string()),
                    parent_index: Some(host_index),
                });
            }
        }
        "macro" => {
            if let Some(name) = read_ident(rest) {
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: format!("{template_name}.{name}"),
                    kind: SymbolKind::Function,
                    visibility: Some(Visibility::Public),
                    start_line: line,
                    end_line: line,
                    start_col: 0,
                    end_col: 0,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some(template_name.to_string()),
                    parent_index: Some(host_index),
                });
            }
        }
        "extends" | "include" | "embed" => {
            if let Some(target) = read_string_arg(rest) {
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: template_name_from_twig_arg(&target),
                    kind: EdgeKind::Imports,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
        "use" | "import" => {
            // `{% use "components/forms.html.twig" %}` — first string arg.
            if let Some(target) = read_string_arg(rest) {
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: template_name_from_twig_arg(&target),
                    kind: EdgeKind::Imports,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
        "from" => {
            // `{% from "macros.html.twig" import foo, bar %}` — first arg
            // is the template; the imports themselves are macro names we
            // don't separately track (they're scoped lookups).
            if let Some(target) = read_string_arg(rest) {
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: template_name_from_twig_arg(&target),
                    kind: EdgeKind::Imports,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
        _ => {}
    }
}

fn read_ident(s: &str) -> Option<String> {
    let trimmed = s.trim_start();
    let end = trimmed
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(trimmed.len());
    if end == 0 { None } else { Some(trimmed[..end].to_string()) }
}

fn read_string_arg(s: &str) -> Option<String> {
    let trimmed = s.trim_start();
    let bytes = trimmed.as_bytes();
    let quote = match bytes.first() {
        Some(b'"') => b'"',
        Some(b'\'') => b'\'',
        _ => return None,
    };
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => break,
            _ => i += 1,
        }
    }
    if i >= bytes.len() { return None; }
    Some(trimmed[1..i].to_string())
}

/// Twig template paths as used in `{% include "x/y.html.twig" %}` map
/// to dotted identifiers `x.y` so they line up with the qualified_name
/// used for the host file symbol.
fn template_name_from_twig_arg(arg: &str) -> String {
    let stem = arg.strip_suffix(".html.twig")
        .or_else(|| arg.strip_suffix(".twig"))
        .unwrap_or(arg);
    stem.replace('/', ".")
}

pub fn template_name_from_path(file_path: &str) -> String {
    let normalized = file_path.replace('\\', "/");
    let stem = match normalized.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/{}", strip_twig_ext(name)),
        None => strip_twig_ext(&normalized).to_string(),
    };
    // Match `/templates/` mid-path OR a leading `templates/` at the
    // path root — Symfony places templates at the project root in
    // `templates/` and `views/`.
    let after = if let Some((_, rest)) = stem.rsplit_once("/templates/") {
        rest.to_string()
    } else if let Some((_, rest)) = stem.rsplit_once("/views/") {
        rest.to_string()
    } else if let Some(rest) = stem.strip_prefix("templates/") {
        rest.to_string()
    } else if let Some(rest) = stem.strip_prefix("views/") {
        rest.to_string()
    } else {
        stem.clone()
    };
    after.replace('/', ".")
}

fn strip_twig_ext(name: &str) -> &str {
    name.strip_suffix(".html.twig")
        .or_else(|| name.strip_suffix(".twig"))
        .or_else(|| Path::new(name).file_stem().and_then(|s| s.to_str()))
        .unwrap_or(name)
}

fn line_at(bytes: &[u8], pos: usize) -> u32 {
    let mut line: u32 = 0;
    for b in bytes.iter().take(pos) {
        if *b == b'\n' { line += 1; }
    }
    line
}

fn has_prefix(bytes: &[u8], start: usize, needle: &[u8]) -> bool {
    if start + needle.len() > bytes.len() { return false; }
    &bytes[start..start + needle.len()] == needle
}

fn find_subseq(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || start > bytes.len() { return None; }
    let end = bytes.len().saturating_sub(needle.len()) + 1;
    (start..end).find(|&i| bytes[i..].starts_with(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_name_strips_views_or_templates_prefix() {
        assert_eq!(
            template_name_from_path("app/templates/users/show.html.twig"),
            "users.show"
        );
        assert_eq!(
            template_name_from_path("themes/foo/templates/page.html.twig"),
            "page"
        );
    }

    #[test]
    fn block_yields_method_symbol() {
        let src = "{% block content %}\n<h1>x</h1>\n{% endblock %}";
        let r = extract(src, "templates/page.html.twig");
        let block = r.symbols.iter().find(|s| s.name == "content")
            .expect("content block missing");
        assert_eq!(block.kind, SymbolKind::Method);
        assert_eq!(block.qualified_name, "page.content");
    }

    #[test]
    fn macro_yields_function_symbol() {
        let src = "{% macro greeting(name) %}Hi {{ name }}{% endmacro %}";
        let r = extract(src, "templates/macros.html.twig");
        let m = r.symbols.iter().find(|s| s.name == "greeting")
            .expect("macro missing");
        assert_eq!(m.kind, SymbolKind::Function);
    }

    #[test]
    fn extends_emits_imports_ref_normalized_to_dotted() {
        let src = "{% extends 'layouts/base.html.twig' %}";
        let r = extract(src, "templates/page.html.twig");
        assert_eq!(r.refs.len(), 1);
        assert_eq!(r.refs[0].target_name, "layouts.base");
        assert_eq!(r.refs[0].kind, EdgeKind::Imports);
    }

    #[test]
    fn include_use_embed_all_emit_refs() {
        let src = "{% include 'partials/header.html.twig' %}\n\
                   {% use 'components/forms.html.twig' %}\n\
                   {% embed 'layouts/card.html.twig' %}{% endembed %}";
        let r = extract(src, "templates/page.html.twig");
        let targets: Vec<&str> = r.refs.iter().map(|rf| rf.target_name.as_str()).collect();
        assert!(targets.contains(&"partials.header"));
        assert!(targets.contains(&"components.forms"));
        assert!(targets.contains(&"layouts.card"));
    }

    #[test]
    fn from_directive_emits_template_ref() {
        let src = "{% from 'macros.html.twig' import greeting %}";
        let r = extract(src, "templates/page.html.twig");
        let m = r.refs.iter().find(|rf| rf.target_name == "macros");
        assert!(m.is_some());
    }

    #[test]
    fn comments_are_skipped() {
        let src = "{# {% block hidden %} #}\n{% block real %}{% endblock %}";
        let r = extract(src, "templates/page.html.twig");
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"real"));
        assert!(!names.contains(&"hidden"));
    }

    #[test]
    fn whitespace_control_markers_handled() {
        // `{%- ... -%}` is the same as `{% ... %}` — whitespace control,
        // not a different tag form.
        let src = "{%- block content -%}\n{%- endblock -%}";
        let r = extract(src, "templates/page.html.twig");
        assert!(r.symbols.iter().any(|s| s.name == "content"));
    }
}
