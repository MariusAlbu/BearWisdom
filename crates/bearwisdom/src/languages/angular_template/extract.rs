//! Host-level extraction for Angular templates. Emits a file-stem
//! symbol + `Calls` refs for every child component tag encountered
//! (PascalCase and kebab-case both accepted — `my-widget` normalized
//! to `MyWidget`). Also emits `Calls` refs for attribute-based
//! directives (`[appHighlight]`, `*ngFor`, `[(ngModel)]`).

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

/// Decide whether a template tag is a standard HTML element (and thus not
/// worth emitting as a component call).
///
/// HTML5 elements are always lowercase and never contain `-`. Angular /
/// web-component elements MUST use kebab-case (`app-nav-menu`,
/// `router-outlet`) — the `-` is mandated by both the custom-elements spec
/// and Angular's component-selector linter. React-in-template conventions
/// use PascalCase. This covers all three cases without a hand-maintained
/// tag list (which would drift — `hgroup`, `search`, `hgroup`, future
/// HTML5 additions, etc. are easy to miss).
///
/// Angular's structural pseudo-elements (`ng-template`, `ng-container`,
/// `ng-content`) contain `-` but aren't components; explicitly skip them.
pub(crate) fn is_standard_html_element(tag: &str) -> bool {
    // Structural Angular pseudo-elements: templated, not component refs.
    if matches!(tag, "ng-template" | "ng-container" | "ng-content") {
        return true;
    }
    // HTML5: lowercase, no `-`. Anything else is a component-looking tag.
    tag.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

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
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        };
    }
    let Some(tree) = parser.parse(source, None) else {
        return ExtractionResult {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: true,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        };
    };

    collect_component_refs(&tree.root_node(), source, host_index, &mut refs);

    ExtractionResult {
        symbols,
        refs,
        routes: Vec::new(),
        db_sets: Vec::new(),
        has_errors: tree.root_node().has_error(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
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
                if !is_standard_html_element(&tag) {
                    // Determine the PascalCase normalized name used as `target_name`.
                    let normalized = if tag.chars().next().map_or(false, |c| c.is_uppercase()) {
                        tag.clone()
                    } else if tag.contains('-') {
                        kebab_to_pascal(&tag)
                    } else {
                        tag.clone()
                    };

                    // For kebab-case tags store the raw tag in `module` so the
                    // `AngularResolver` can look it up in the project-wide selector
                    // map built from `@Component({selector:'...'})` metadata.
                    // When the resolver finds a match it replaces `target_name`
                    // with the real class qname. When no match is found the existing
                    // `kebab_to_pascal` fallback in `target_name` is still used.
                    let raw_selector = if tag.contains('-') || !tag.chars().next().map_or(false, |c| c.is_uppercase()) {
                        Some(tag.clone())
                    } else {
                        None
                    };

                    refs.push(ExtractedRef {
                        source_symbol_index: host_index,
                        target_name: normalized,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: raw_selector,
                        chain: None,
                        byte_offset: 0,
                        namespace_segments: Vec::new(),
                    });
                }
            }

            // Attribute directives — emit a Calls ref for every attribute
            // whose name looks like an Angular directive selector:
            //   - camelCase names without `-` (e.g. `appHighlight`, `ngFor`)
            //   - Angular structural directive prefix: `*ngFor`, `*ngIf`
            //   - Two-way binding: `[(ngModel)]` → normalizes to `ngModel`
            //   - Property binding: `[ngClass]` → normalizes to `ngClass`
            //
            // Standard HTML attributes (all-lowercase, no camelCase, no special
            // prefix) are skipped to avoid false positives.
            collect_attribute_directive_refs(&child, source, host_index, refs);
        }
        collect_component_refs(&child, source, host_index, refs);
    }
}

/// Walk the start_tag / self_closing_tag children of an element and emit
/// `Calls` refs for attribute names that look like Angular directive selectors.
fn collect_attribute_directive_refs(
    element: &Node,
    source: &str,
    host_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = element.walk();
    for child in element.children(&mut cursor) {
        let ck = child.kind();
        if !matches!(ck, "start_tag" | "self_closing_tag") {
            continue;
        }
        let mut ac = child.walk();
        for attr in child.children(&mut ac) {
            if attr.kind() != "attribute" {
                continue;
            }
            // tree-sitter-html uses `attribute_name` as the child node kind,
            // not as a named field — use child_by_field_name("name") first and
            // fall back to walking children for the `attribute_name` node.
            let raw_attr: &str = {
                if let Some(n) = attr.child_by_field_name("name") {
                    source.get(n.start_byte()..n.end_byte()).unwrap_or("")
                } else {
                    let mut found = "";
                    let mut nc = attr.walk();
                    for n in attr.children(&mut nc) {
                        if n.kind() == "attribute_name" {
                            found = source.get(n.start_byte()..n.end_byte()).unwrap_or("");
                            break;
                        }
                    }
                    found
                }
            };
            if let Some(selector) = normalize_attribute_as_directive(raw_attr) {
                refs.push(ExtractedRef {
                    source_symbol_index: host_index,
                    target_name: selector.clone(),
                    kind: EdgeKind::Calls,
                    line: attr.start_position().row as u32,
                    // Raw selector stored in `module` for resolver lookup.
                    module: Some(selector),
                    chain: None,
                    byte_offset: 0,
                    namespace_segments: Vec::new(),
                });
            }
        }
    }
}

/// Decide whether an HTML attribute name looks like an Angular directive
/// selector and return its normalized form, or `None` if it's a plain
/// HTML attribute.
///
/// Rules:
/// - `*ngFor`, `*ngIf`, `*ngSwitchCase` → strip `*` prefix → `ngFor`, `ngIf`
/// - `[(ngModel)]` → strip `[(` and `)]` → `ngModel`
/// - `[ngClass]`, `[appHighlight]` → strip `[` and `]`
/// - `(click)`, `(submit)` → event bindings — skip (these are DOM events not directives)
/// - `appHighlight`, `ngFor` (without brackets) → keep as-is if camelCase
/// - All-lowercase / hyphenated standard HTML attrs → `None`
///
/// Only emits refs for camelCase names (has at least one uppercase letter)
/// or names with the `ng`/`app`/`lib` prefix pattern to avoid flooding with
/// standard HTML attributes like `class`, `href`, `data-*`.
pub(crate) fn normalize_attribute_as_directive(raw: &str) -> Option<String> {
    // Strip Angular binding wrappers:
    let name = if raw.starts_with("[(") && raw.ends_with(")]") {
        // Two-way binding `[(ngModel)]` → `ngModel`
        &raw[2..raw.len() - 2]
    } else if raw.starts_with('[') && raw.ends_with(']') {
        // Property binding `[ngClass]` → `ngClass`
        // Skip if value looks like a native HTML attr (all lowercase).
        let inner = &raw[1..raw.len() - 1];
        if inner.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
            return None;
        }
        inner
    } else if raw.starts_with('(') && raw.ends_with(')') {
        // Event binding `(click)` — DOM events, skip.
        return None;
    } else if let Some(stripped) = raw.strip_prefix('*') {
        // Structural directive `*ngFor` → `ngFor`
        stripped
    } else {
        // Bare attribute name — only keep if camelCase.
        raw
    };

    // Only emit for names with at least one uppercase letter (camelCase),
    // or names that start with standard Angular/community prefixes.
    let has_upper = name.chars().any(|c| c.is_ascii_uppercase());
    let has_ng_prefix = name.starts_with("ng")
        || name.starts_with("app")
        || name.starts_with("lib")
        || name.starts_with("cdk")
        || name.starts_with("mat");

    if name.is_empty() || (!has_upper && !has_ng_prefix) {
        return None;
    }

    Some(name.to_string())
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

pub(crate) fn kebab_to_pascal(s: &str) -> String {
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
#[path = "extract_tests.rs"]
mod tests;
