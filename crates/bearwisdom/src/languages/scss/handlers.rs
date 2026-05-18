//! Per-node handlers for the SCSS tree-sitter walker.
//!
//! `visit_node` dispatches by tree-sitter node kind to a handler that emits
//! the matching symbol or ref. Handlers may recurse via `visit_children` to
//! descend into nested blocks.

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

use super::extract::{
    extract_all_selector_names, extract_selector_base_name, find_child_of_kind,
    find_include_target, find_selector_target, find_string_value, find_use_alias,
    make_sym, node_text, path_to_target, SCSS_CSS_FN_HINT,
};

// ---------------------------------------------------------------------------
// Tree walker — dispatches on SCSS grammar node kinds
// ---------------------------------------------------------------------------

pub(super) fn visit_node(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    match node.kind() {
        "mixin_statement" => {
            handle_mixin(node, src, symbols, refs);
        }
        "function_statement" => {
            handle_function(node, src, symbols, refs);
        }
        "include_statement" => {
            let sym_idx = symbols.len();
            handle_include(node, src, refs, symbols, sym_idx);
        }
        "extend_statement" => {
            handle_extend(node, src, refs, symbols.len());
        }
        "import_statement" => {
            let sym_idx = symbols.len();
            handle_import(node, src, refs, symbols, sym_idx);
        }
        "forward_statement" => {
            let sym_idx = symbols.len();
            handle_forward(node, src, refs, symbols, sym_idx);
        }
        "use_statement" => {
            let sym_idx = symbols.len();
            handle_use(node, src, refs, symbols, sym_idx);
        }
        "keyframes_statement" => {
            handle_keyframes(node, src, symbols, refs);
        }
        "rule_set" => {
            handle_rule_set(node, src, symbols, refs, parent_idx);
        }
        "declaration" => {
            handle_declaration(node, src, symbols, refs, parent_idx);
        }
        "call_expression" => {
            let sym_idx = symbols.len();
            handle_call_expr(node, src, refs, symbols, sym_idx);
        }
        _ => {
            // Recurse into all other nodes (stylesheet, block, media_statement, etc.)
            visit_children(node, src, symbols, refs, parent_idx);
        }
    }
}

fn visit_children(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            visit_node(&child, src, symbols, refs, parent_idx);
        }
    }
}

// ---------------------------------------------------------------------------
// @mixin name { ... }  =>  Function symbol + recurse body
// ---------------------------------------------------------------------------

fn handle_mixin(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }

    let idx = symbols.len();
    symbols.push(make_sym(
        name.clone(),
        SymbolKind::Function,
        node,
        None,
        Some(format!("@mixin {name}")),
    ));

    // Recurse into all children (parameters with defaults, block body)
    visit_children(node, src, symbols, refs, Some(idx));
}

// ---------------------------------------------------------------------------
// @function name($args) { ... }  =>  Function symbol + recurse body
// ---------------------------------------------------------------------------

fn handle_function(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(n, src))
        .unwrap_or_default();
    if name.is_empty() {
        return;
    }

    let idx = symbols.len();
    symbols.push(make_sym(
        name.clone(),
        SymbolKind::Function,
        node,
        None,
        Some(format!("@function {name}")),
    ));

    // Recurse into all children (parameters with defaults, block body)
    visit_children(node, src, symbols, refs, Some(idx));
}

// ---------------------------------------------------------------------------
// @include mixin-name(args)  =>  Calls ref
// ---------------------------------------------------------------------------

fn handle_include(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    source_symbol_index: usize,
) {
    // `find_include_target` inspects raw node text to detect the
    // `namespace.mixin` dotted form that the SCSS grammar collapses into a
    // single identifier. When a dot is present the namespace prefix is used
    // as the target so the resolver can match it against `@use` alias entries
    // and classify the call as external.
    let target = find_include_target(node, src);
    if !target.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
    // Recurse into arguments to find nested call_expressions
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}

// ---------------------------------------------------------------------------
// @extend .selector / %placeholder  =>  Inherits ref
// ---------------------------------------------------------------------------

fn handle_extend(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    source_symbol_index: usize,
) {
    let target = find_selector_target(node, src);
    if target.is_empty() {
        return;
    }
    // Interpolated selectors (`@extend .#{$expr}`) are dynamic and cannot
    // be resolved statically — skip rather than emit an unresolvable ref.
    if target.contains("#{") {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: target,
        kind: EdgeKind::Inherits,
        line: node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

// ---------------------------------------------------------------------------
// @import 'path'  =>  Imports ref
// ---------------------------------------------------------------------------

fn handle_import(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    source_symbol_index: usize,
) {
    let module = find_string_value(node, src);
    if !module.is_empty() {
        let target = path_to_target(&module);
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            col: 0,
            module: Some(module),
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}

// ---------------------------------------------------------------------------
// @forward 'path'  =>  Imports ref
// ---------------------------------------------------------------------------

fn handle_forward(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    source_symbol_index: usize,
) {
    let module = find_string_value(node, src);
    if !module.is_empty() {
        let target = path_to_target(&module);
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            col: 0,
            module: Some(module),
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}

// ---------------------------------------------------------------------------
// @use 'path'  =>  Imports ref
// ---------------------------------------------------------------------------

fn handle_use(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    source_symbol_index: usize,
) {
    let module = find_string_value(node, src);
    if !module.is_empty() {
        // When `@use 'path' as alias` is present, store the alias as the
        // target_name so that `@include alias.mixin()` calls can be matched
        // back to this import entry via the alias field in FileContext.
        //
        // For `@use 'sass:math'` (no `as` clause), Sass introduces the
        // namespace `math` — the segment after the colon. `path_to_target`
        // would return `"sass:math"` which doesn't match `math` used as a
        // namespace prefix, so we strip the `sass:` prefix explicitly.
        let alias = find_use_alias(node, src);
        let target = if !alias.is_empty() {
            alias.clone()
        } else if let Some(stem) = module.strip_prefix("sass:") {
            stem.to_string()
        } else {
            path_to_target(&module)
        };
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            col: 0,
            module: Some(module),
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}

// ---------------------------------------------------------------------------
// @keyframes name { ... }  =>  Function symbol
// ---------------------------------------------------------------------------

fn handle_keyframes(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = find_child_of_kind(node, "keyframes_name")
        .map(|n| node_text(n, src))
        .or_else(|| find_child_of_kind(node, "identifier").map(|n| node_text(n, src)))
        .unwrap_or_default();

    let idx = symbols.len();
    if !name.is_empty() {
        symbols.push(make_sym(
            name.clone(),
            SymbolKind::Function,
            node,
            None,
            Some(format!("@keyframes {name}")),
        ));
    }

    // Recurse into keyframe_block_list for any nested call_expressions
    visit_children(node, src, symbols, refs, Some(idx));
}

// ---------------------------------------------------------------------------
// rule_set { selectors { block } }  =>  Class symbol per selector
//
// A single rule_set can have multiple comma-separated selectors
// (`.container, .container-fluid { ... }`) — emit one Class symbol per
// distinct base name so that `@extend` and `Inherits` refs can resolve
// to any of them. Compound selectors (`.button.button-assertive`) contribute
// each chained class individually. Pseudo-element / pseudo-class suffixes
// (`:before`, `:after`, `:hover`) are stripped so `.clearfix:before` and
// `.clearfix:after` both produce the base name `clearfix`, which matches
// an `@extend .clearfix` that would otherwise be unresolvable.
// ---------------------------------------------------------------------------

fn handle_rule_set(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    // Collect all distinct base names from the selector list.
    let names: Vec<String> = if let Some(sel_node) = find_child_of_kind(node, "selectors") {
        extract_all_selector_names(&sel_node, src)
    } else {
        // Fallback: parse the raw source line when the grammar didn't produce
        // a `selectors` node (e.g. after a parse error or for unusual rules).
        let row = node.start_position().row;
        src.lines()
            .nth(row)
            .into_iter()
            .flat_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .split(|c: char| c == '{' || c == ',' || c == ' ')
                    .map(|seg| {
                        seg.trim_start_matches('.')
                            .trim_start_matches('#')
                            .trim_start_matches('%')
                            .trim_start_matches('&')
                            .to_string()
                    })
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .collect()
    };

    if names.is_empty() {
        visit_children(node, src, symbols, refs, parent_idx);
        return;
    }

    // Emit one symbol per unique base name. The first emitted symbol owns
    // the parent-index slot used by child rule_sets.
    let first_idx = symbols.len();
    let mut emitted_names: Vec<String> = Vec::new();
    for name in names {
        if emitted_names.contains(&name) {
            continue;
        }
        emitted_names.push(name.clone());
        symbols.push(make_sym(name, SymbolKind::Class, node, parent_idx, None));
    }

    // Recurse into all children (selectors may contain pseudo-class call_expressions,
    // block contains nested rules and declarations)
    visit_children(node, src, symbols, refs, Some(first_idx));
}

// ---------------------------------------------------------------------------
// declaration: $variable: value  =>  Variable symbol
// ---------------------------------------------------------------------------

fn handle_declaration(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    // First child of declaration is the property_name (or variable).
    // If it starts with $ it's an SCSS variable declaration.
    if let Some(prop) = node.child(0) {
        let raw = node_text(prop, src);
        if raw.starts_with('$') {
            let name = raw.trim_start_matches('$').to_string();
            if !name.is_empty() {
                let first_line = src
                    .lines()
                    .nth(node.start_position().row)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                symbols.push(make_sym(
                    name,
                    SymbolKind::Variable,
                    node,
                    parent_idx,
                    Some(first_line),
                ));
            }
            // Still recurse to find call_expressions in the value
        }
    }
    // Recurse into all children to find nested call_expressions and refs
    visit_children(node, src, symbols, refs, parent_idx);
}

// ---------------------------------------------------------------------------
// call_expression  =>  Calls ref
// ---------------------------------------------------------------------------

fn handle_call_expr(
    node: &Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    symbols: &mut Vec<ExtractedSymbol>,
    source_symbol_index: usize,
) {
    // Extract the function name from the call_expression node.
    // The function_name child is a leaf with the function identifier text.
    let func_name = find_child_of_kind(node, "function_name")
        .map(|n| node_text(n, src))
        .or_else(|| node.child(0).map(|n| {
            let t = node_text(n, src);
            // Extract identifier from interpolation or other non-leaf
            t.trim_matches('#').trim_matches('{').trim_matches('}').to_string()
        }))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "<call>".to_string());

    let target = func_name
        .rsplit('.')
        .next()
        .unwrap_or(&func_name)
        .trim()
        .to_string();

    let target = if target.is_empty() {
        "<call>".to_string()
    } else {
        target
    };

    // Emit a Calls ref tagged as a property-value function call (via the
    // `module` hint below). The resolver uses this to distinguish CSS/SCSS
    // built-in function evaluation (`rgb(…)`, `calc(…)`, `color-mix(…)`,
    // `steps(…)`, `oklch(…)`, `map-get(…)` …) from user-defined
    // `@include mixin-name(…)` calls. Without the hint, the resolver
    // would either have to maintain a drifting hardcoded list of CSS
    // built-ins (which misses every new CSS Level 5+ addition) or
    // treat all unresolved calls as external, which hides genuinely
    // broken `@include` references.
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: target,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        col: 0,
        module: Some(SCSS_CSS_FN_HINT.to_string()),
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });

    // Recurse into children (arguments may contain nested call_expressions).
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}
