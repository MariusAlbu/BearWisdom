// =============================================================================
// languages/scss/extract.rs  —  SCSS / Sass extractor
//
// Grammar: tree-sitter-scss-local (dedicated SCSS grammar, MSVC-compatible
//   via pre-expanded parser_expanded.c). The SCSS grammar has proper nodes
//   for every SCSS construct; no CSS grammar fallback needed.
//
// SYMBOLS:
//   Function  — mixin_statement, function_statement, keyframes_statement
//   Class     — rule_set (selectors)
//   Variable  — declaration with $variable LHS
//
// REFERENCES:
//   Calls     — include_statement, call_expression
//   Inherits  — extend_statement
//   Imports   — import_statement, forward_statement
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::{Node, Parser};

use super::handlers::visit_node;
use super::recovery::{
    recover_class_symbols_from_text, recover_mixin_symbols_from_text,
    recover_sass_indented_symbols_from_text,
};

/// Tag placed in `ExtractedRef.module` to mark property-value
/// `call_expression`-derived Calls refs. The resolver treats these as
/// CSS/SCSS built-in function evaluation rather than user-defined mixin
/// calls. Public so the resolver can import the same constant.
pub(crate) const SCSS_CSS_FN_HINT: &str = "__scss_css_fn__";

pub fn extract(source: &str, file_path: &str) -> super::ExtractionResult {
    // Indented-syntax `.sass` files are not handled by the SCSS grammar;
    // fall back to a text-only scan that recognises `=mixin-name` and
    // `@mixin mixin-name` declarations.
    if file_path.ends_with(".sass") {
        let mut symbols: Vec<ExtractedSymbol> = Vec::new();
        let refs: Vec<ExtractedRef> = Vec::new();
        recover_mixin_symbols_from_text(source, &mut symbols);
        recover_sass_indented_symbols_from_text(source, &mut symbols);
        return super::ExtractionResult::new(symbols, refs, true);
    }

    let language: tree_sitter::Language = tree_sitter_scss_local::LANGUAGE.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load SCSS grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    let root = tree.root_node();
    visit_node(&root, source, &mut symbols, &mut refs, None);

    // Error-recovery fallback: tree-sitter-scss-local degrades to a root
    // `ERROR` node for any file containing a construct the grammar can't
    // handle (e.g. `#{$a}/#{$b}` interpolations in a `font:` shorthand,
    // or `@mixin name()` with empty parens). Run the text-scan fallback
    // whenever the tree has errors — not just when it found zero symbols —
    // so that mixins defined after the first parse error are also captured.
    // The `already` guard in the scan prevents double-emission for any
    // symbol the grammar-driven path already found.
    if has_errors {
        recover_mixin_symbols_from_text(source, &mut symbols);
        recover_class_symbols_from_text(source, &mut symbols);
    }

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn make_sym(
    name: String,
    kind: SymbolKind,
    node: &Node,
    parent_index: Option<usize>,
    signature: Option<String>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: None,
        parent_index,
        byte_offset: node.start_byte() as u32,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

pub(super) fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

pub(super) fn find_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

/// Returns the include target for `@include [ns.]name(…)`.
///
/// The SCSS grammar does not model namespace-qualified includes as two
/// separate identifier nodes — it surfaces only the leading part before the
/// first dot. Raw-text inspection of the first child's source span detects
/// the dot and returns the namespace prefix so the resolver can match it
/// against `@use` alias entries.
pub(super) fn find_include_target(node: &Node, src: &str) -> String {
    // The grammar emits the mixin name (or the namespace prefix for
    // dotted forms) as the first `identifier` child.
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "identifier" {
                let text = node_text(child, src);
                // Peek at the byte immediately following the identifier in
                // the raw source to detect `namespace.mixin` form.
                let end_byte = child.end_byte();
                if end_byte < src.len() && src.as_bytes().get(end_byte) == Some(&b'.') {
                    // Dotted form: return the namespace prefix so the
                    // resolver can classify this as a module-qualified call.
                    return text;
                }
                return text;
            }
        }
    }
    String::new()
}

/// Extracts the `as alias` clause from a `@use 'path' as alias` statement.
///
/// The SCSS grammar does not model the `as alias` syntax — it produces an
/// `ERROR` node for the entire `as alias` token sequence. Raw-text scanning
/// of the node's source span is the only reliable approach.
///
/// Returns the alias string, or an empty string if no `as` clause is present.
pub(super) fn find_use_alias(node: &Node, src: &str) -> String {
    let raw = node_text(*node, src);
    // Match ` as <identifier>` anywhere in the statement, stopping at `;`,
    // whitespace, or end of input. The `as` keyword is lower-case in SCSS.
    if let Some(idx) = raw.find(" as ") {
        let rest = &raw[idx + 4..];
        let alias: String = rest
            .chars()
            .take_while(|&c| c.is_alphanumeric() || c == '_' || c == '-')
            .collect();
        if !alias.is_empty() {
            return alias;
        }
    }
    String::new()
}

pub(super) fn find_selector_target(node: &Node, src: &str) -> String {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "class_selector" => {
                    if let Some(cn) = child
                        .child_by_field_name("class_name")
                        .or_else(|| child.child(1))
                    {
                        return node_text(cn, src);
                    }
                }
                "placeholder" => {
                    if let Some(cn) = child.child(1) {
                        return node_text(cn, src);
                    }
                }
                "identifier" => {
                    return node_text(child, src);
                }
                _ => {}
            }
        }
    }
    String::new()
}

pub(super) fn find_string_value(node: &Node, src: &str) -> String {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "string_value" {
                let raw = node_text(child, src);
                return raw.trim_matches('"').trim_matches('\'').to_string();
            }
        }
    }
    String::new()
}

pub(super) fn path_to_target(module: &str) -> String {
    module
        .rsplit('/')
        .next()
        .unwrap_or(module)
        .trim_start_matches('_')
        .trim_end_matches(".scss")
        .trim_end_matches(".sass")
        .trim_end_matches(".css")
        .to_string()
}

/// Extract the base name from a single selector node, stripping pseudo-elements
/// and pseudo-classes. Returns the canonical form (`.name`, `#name`, `%name`,
/// or bare tag name) or `None` when the node has no extractable name.
///
/// Pseudo suffixes (`:before`, `:after`, `:hover`, `::placeholder`) are
/// intentionally dropped so `.clearfix:before` and `.clearfix:after` both
/// produce `clearfix`. This lets an `@extend .clearfix` resolve to either
/// pseudo rule definition where no standalone `.clearfix {}` rule exists.
pub(super) fn extract_selector_base_name(child: &Node, src: &str) -> Option<String> {
    match child.kind() {
        "class_selector" => {
            // Strip any leading chained classes — for `.button.button-assertive`
            // the grammar nests the second class inside the first as a child.
            // We collect all chained classes and return them via the caller.
            let name = child
                .child_by_field_name("class_name")
                .or_else(|| child.child(1))
                .map(|n| node_text(n, src))?;
            if !name.is_empty() {
                Some(name)
            } else {
                None
            }
        }
        "id_selector" => {
            let name = child
                .child_by_field_name("id_name")
                .or_else(|| child.child(1))
                .map(|n| node_text(n, src))?;
            if !name.is_empty() {
                Some(name)
            } else {
                None
            }
        }
        "placeholder" => {
            let name = child.child(1).map(|n| node_text(n, src))?;
            if !name.is_empty() {
                Some(name)
            } else {
                None
            }
        }
        "tag_name" | "nesting_selector" | "universal_selector" => {
            let t = node_text(*child, src);
            if !t.is_empty() {
                Some(t)
            } else {
                None
            }
        }
        // Pseudo-class / pseudo-element selectors that appear as standalone
        // rule starters (`:root`, `::before` at the top level) are kept.
        // Pseudo annotations on class selectors are stripped by the class_selector
        // arm — they appear as sibling nodes in the grammar, not children.
        "pseudo_class_selector" | "pseudo_element_selector" => {
            let t = node_text(*child, src);
            if !t.is_empty() && !t.contains('{') {
                Some(t)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Collect all distinct base class names from a `selectors` node.
///
/// Handles three patterns:
/// 1. Comma list: `.a, .b { }` — walk all top-level selector children.
/// 2. Compound: `.button.button-assertive { }` — the grammar nests the second
///    `class_selector` inside the first. Walk inner children of each
///    `class_selector` to pick up chained classes.
/// 3. Pseudo suffix: `.clearfix:before, .clearfix:after { }` — pseudo children
///    that follow a class name are silently skipped; only the class name is
///    emitted. Two pseudo rules for the same base class produce one name
///    (deduplication is in the caller).
pub(super) fn extract_all_selector_names(node: &Node, src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for i in 0..node.child_count() {
        let Some(child) = node.child(i) else { continue };
        match child.kind() {
            "class_selector" => {
                // The primary class name.
                if let Some(name) = child
                    .child_by_field_name("class_name")
                    .or_else(|| child.child(1))
                    .map(|n| node_text(n, src))
                    .filter(|s| !s.is_empty())
                {
                    if !out.contains(&name) {
                        out.push(name);
                    }
                }
                // Chained classes inside the same class_selector node
                // (`.button.button-assertive` produces a nested class_selector
                // for `.button-assertive` as a child of the outer one).
                for j in 0..child.child_count() {
                    let Some(inner) = child.child(j) else {
                        continue;
                    };
                    if inner.kind() == "class_selector" {
                        if let Some(inner_name) = inner
                            .child_by_field_name("class_name")
                            .or_else(|| inner.child(1))
                            .map(|n| node_text(n, src))
                            .filter(|s| !s.is_empty())
                        {
                            if !out.contains(&inner_name) {
                                out.push(inner_name);
                            }
                        }
                    }
                }
            }
            "id_selector" | "placeholder" | "tag_name" | "nesting_selector"
            | "universal_selector" => {
                if let Some(name) = extract_selector_base_name(&child, src) {
                    if !out.contains(&name) {
                        out.push(name);
                    }
                }
            }
            // Pseudo selectors can appear in two ways:
            // (a) `.clearfix:before` — the grammar wraps the class selector
            //     inside a pseudo_class_selector; look for the inner
            //     class_selector and extract its name to get "clearfix".
            // (b) `:root` — a standalone pseudo with no inner class; extract
            //     its text verbatim as the selector name.
            "pseudo_class_selector" | "pseudo_element_selector" => {
                // Probe for a nested class_selector (case a).
                let mut found_inner = false;
                for j in 0..child.child_count() {
                    let Some(inner) = child.child(j) else {
                        continue;
                    };
                    if inner.kind() == "class_selector" {
                        if let Some(name) = inner
                            .child_by_field_name("class_name")
                            .or_else(|| inner.child(1))
                            .map(|n| node_text(n, src))
                            .filter(|s| !s.is_empty())
                        {
                            if !out.contains(&name) {
                                out.push(name);
                            }
                            found_inner = true;
                        }
                    }
                }
                // Case (b): standalone pseudo like `:root`.
                if !found_inner {
                    let t = node_text(child, src);
                    if !t.is_empty() && !t.contains('{') && !out.contains(&t) {
                        out.push(t);
                    }
                }
            }
            // Commas and whitespace — skip.
            _ => {}
        }
    }

    out
}
