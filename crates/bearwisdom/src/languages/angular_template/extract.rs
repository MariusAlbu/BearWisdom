//! Host-level extraction for Angular templates. Emits a file-stem
//! symbol + `Calls` refs for every child component tag encountered
//! (PascalCase and kebab-case both accepted — `my-widget` normalized
//! to `MyWidget`).

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

const HTML_BUILTINS: &[&str] = &[
    "html", "head", "body", "title", "meta", "link", "script", "style",
    "div", "span", "p", "a", "img", "ul", "ol", "li", "table", "tr", "td", "th",
    "thead", "tbody", "tfoot", "form", "input", "button", "select", "option",
    "textarea", "label", "fieldset", "legend", "h1", "h2", "h3", "h4", "h5", "h6",
    "header", "footer", "nav", "section", "article", "aside", "main",
    "strong", "em", "b", "i", "u", "code", "pre", "blockquote", "hr", "br",
    "svg", "path", "circle", "rect", "g", "use", "defs", "symbol",
    "canvas", "video", "audio", "source", "track", "picture",
    "iframe", "embed", "object", "param", "details", "summary", "figure", "figcaption",
    "template", "slot", "ng-template", "ng-container", "ng-content",
];

pub fn extract(source: &str, file_path: &str) -> ExtractionResult {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let file_name = file_stem(file_path);
    symbols.push(ExtractedSymbol {
        name: file_name.clone(),
        qualified_name: file_name.clone(),
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
    let host_index = 0usize;

    let language: tree_sitter::Language = tree_sitter_html::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return ExtractionResult {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: true,
        };
    }
    let Some(tree) = parser.parse(source, None) else {
        return ExtractionResult {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: true,
        };
    };

    collect_component_refs(&tree.root_node(), source, host_index, &mut refs);

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: tree.root_node().has_error(),
    }
}

fn collect_component_refs(
    node: &Node,
    source: &str,
    host_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if matches!(kind, "element" | "self_closing_element") {
            if let Some(tag) = element_tag_name(&child, source) {
                if !HTML_BUILTINS.contains(&tag.to_ascii_lowercase().as_str()) {
                    let normalized = if tag.chars().next().map_or(false, |c| c.is_uppercase()) {
                        tag.clone()
                    } else if tag.contains('-') {
                        kebab_to_pascal(&tag)
                    } else {
                        tag.clone()
                    };
                    refs.push(ExtractedRef {
                        source_symbol_index: host_index,
                        target_name: normalized,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
        collect_component_refs(&child, source, host_index, refs);
    }
}

fn element_tag_name(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "start_tag" | "self_closing_tag" => {
                let mut tc = child.walk();
                for c in child.children(&mut tc) {
                    if c.kind() == "tag_name" {
                        return src.get(c.start_byte()..c.end_byte()).map(str::to_string);
                    }
                }
            }
            "tag_name" => {
                return src.get(child.start_byte()..child.end_byte()).map(str::to_string);
            }
            _ => {}
        }
    }
    None
}

fn kebab_to_pascal(s: &str) -> String {
    s.split('-')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}

fn file_stem(file_path: &str) -> String {
    let norm = file_path.replace('\\', "/");
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    // `.component.html` is a compound extension — strip both parts.
    if let Some(idx) = name.find(".component.html") {
        return name[..idx].to_string();
    }
    if let Some(idx) = name.find(".container.html") {
        return name[..idx].to_string();
    }
    if let Some(idx) = name.find(".dialog.html") {
        return name[..idx].to_string();
    }
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stem_strips_component_suffix() {
        assert_eq!(file_stem("src/app/user.component.html"), "user");
        assert_eq!(file_stem("foo.dialog.html"), "foo");
    }

    #[test]
    fn pascal_component_tag_becomes_calls_ref() {
        let src = r#"<div><UserCard name="x" /></div>"#;
        let r = extract(src, "parent.component.html");
        let calls: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
        assert_eq!(calls, vec!["UserCard"]);
    }

    #[test]
    fn kebab_tag_normalizes_to_pascal() {
        let src = "<app-user-card></app-user-card>";
        let r = extract(src, "parent.component.html");
        let calls: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
        assert_eq!(calls, vec!["AppUserCard"]);
    }

    #[test]
    fn html_builtins_not_emitted() {
        let src = "<div><p>text</p></div>";
        let r = extract(src, "x.component.html");
        assert!(r.refs.is_empty());
    }

    #[test]
    fn ng_container_ignored_as_builtin() {
        let src = "<ng-container><p>x</p></ng-container>";
        let r = extract(src, "x.component.html");
        assert!(r.refs.is_empty());
    }
}
