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
    }

    super::ExtractionResult::new(symbols, refs, has_errors)
}

/// Byte-level scan for `@mixin NAME` / `@function NAME` declarations.
/// Called only when the tree-sitter parse fails catastrophically and
/// zero structured symbols were extracted — a defensible fallback, not
/// a replacement for the grammar-driven path.
fn recover_mixin_symbols_from_text(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    for (kind_label, at_keyword) in [("@mixin", "@mixin"), ("@function", "@function")] {
        let mut line_no: u32 = 0;
        let mut last_nl: usize = 0;
        let bytes = source.as_bytes();
        let kw = at_keyword.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                line_no += 1;
                last_nl = i + 1;
                i += 1;
                continue;
            }
            // Match the @keyword only at a line start (after whitespace) so
            // we don't pick up `@mixin` appearing in a selector string.
            if bytes[i] == b'@' && bytes.len() - i >= kw.len() && &bytes[i..i + kw.len()] == kw {
                // Require that the previous non-space char on the line is
                // either nothing (start of line) or whitespace — i.e. this
                // `@` begins a statement.
                let mut j = i;
                while j > last_nl {
                    let prev = bytes[j - 1];
                    if prev == b' ' || prev == b'\t' {
                        j -= 1;
                    } else {
                        break;
                    }
                }
                if j != last_nl {
                    i += 1;
                    continue;
                }
                // Skip the keyword and any trailing whitespace.
                let mut k = i + kw.len();
                while k < bytes.len() && (bytes[k] == b' ' || bytes[k] == b'\t') {
                    k += 1;
                }
                // Name runs until `(`, `{`, whitespace, or end-of-line.
                let name_start = k;
                while k < bytes.len()
                    && bytes[k] != b'('
                    && bytes[k] != b'{'
                    && bytes[k] != b' '
                    && bytes[k] != b'\t'
                    && bytes[k] != b'\r'
                    && bytes[k] != b'\n'
                {
                    k += 1;
                }
                if k > name_start {
                    let name = &source[name_start..k];
                    // Skip if already captured via the grammar path (the
                    // guard at call-site ensures the first pass emitted
                    // zero symbols, so this is cheap insurance).
                    let already = symbols.iter().any(|s| s.name == name);
                    if !already {
                        symbols.push(ExtractedSymbol {
                            name: name.to_string(),
                            qualified_name: name.to_string(),
                            kind: SymbolKind::Function,
                            visibility: Some(Visibility::Public),
                            start_line: line_no,
                            end_line: line_no,
                            start_col: (i - last_nl) as u32,
                            end_col: (k - last_nl) as u32,
                            signature: Some(format!("{kind_label} {name}")),
                            doc_comment: None,
                            scope_path: None,
                            parent_index: None,
                        });
                    }
                }
                i = k;
                continue;
            }
            i += 1;
        }
    }
}

/// Text-scan for indented Sass `=mixin-name` declarations.
///
/// The indented Sass syntax uses `=name` for mixin definitions instead of
/// `@mixin name { }`. The SCSS grammar does not handle this form, so `.sass`
/// files run this scan alongside `recover_mixin_symbols_from_text`.
fn recover_sass_indented_symbols_from_text(source: &str, symbols: &mut Vec<ExtractedSymbol>) {
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('=') {
            continue;
        }
        let rest = &trimmed[1..];
        // Name runs until `(`, whitespace, or end-of-line.
        let name: String = rest
            .chars()
            .take_while(|&c| c != '(' && c != ' ' && c != '\t' && c != '\r')
            .collect();
        if name.is_empty() {
            continue;
        }
        let already = symbols.iter().any(|s| s.name == name);
        if !already {
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name.clone(),
                kind: SymbolKind::Function,
                visibility: Some(Visibility::Public),
                start_line: line_no as u32,
                end_line: line_no as u32,
                start_col: (line.len() - trimmed.len()) as u32,
                end_col: (line.len() - trimmed.len() + 1 + name.len()) as u32,
                signature: Some(format!("={name}")),
                doc_comment: None,
                scope_path: None,
                parent_index: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tree walker — dispatches on SCSS grammar node kinds
// ---------------------------------------------------------------------------

fn visit_node(
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
            module: None,
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
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
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
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
            module: Some(module),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
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
            module: Some(module),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
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
            module: Some(module),
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
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
// rule_set { selectors { block } }  =>  Class symbol per rule set
// ---------------------------------------------------------------------------

fn handle_rule_set(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let name = find_child_of_kind(node, "selectors")
        .and_then(|sel| extract_first_selector_name(&sel, src))
        .or_else(|| {
            let row = node.start_position().row;
            src.lines().nth(row).and_then(|line| {
                let trimmed = line.trim();
                let name = trimmed
                    .split(|c: char| c == '{' || c == ',' || c == ' ')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('.')
                    .trim_start_matches('#')
                    .trim_start_matches('%')
                    .trim_start_matches('&');
                if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                }
            })
        });

    let idx = symbols.len();
    if let Some(name) = name {
        let clean = name
            .trim_start_matches('.')
            .trim_start_matches('#')
            .trim_start_matches('%')
            .trim_start_matches('&')
            .to_string();
        let display = if clean.is_empty() { name } else { clean };
        symbols.push(make_sym(display, SymbolKind::Class, node, parent_idx, None));
    } else {
        visit_children(node, src, symbols, refs, parent_idx);
        return;
    }

    // Recurse into all children (selectors may contain pseudo-class call_expressions,
    // block contains nested rules and declarations)
    visit_children(node, src, symbols, refs, Some(idx));
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
        module: Some(SCSS_CSS_FN_HINT.to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    });

    // Recurse into children (arguments may contain nested call_expressions).
    visit_children(node, src, symbols, refs, Some(source_symbol_index));
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_sym(
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
    }
}

fn node_text(node: Node, src: &str) -> String {
    src[node.start_byte()..node.end_byte()].to_string()
}

fn find_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
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
fn find_include_target(node: &Node, src: &str) -> String {
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
fn find_use_alias(node: &Node, src: &str) -> String {
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

fn find_selector_target(node: &Node, src: &str) -> String {
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

fn find_string_value(node: &Node, src: &str) -> String {
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

fn path_to_target(module: &str) -> String {
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

fn extract_first_selector_name(node: &Node, src: &str) -> Option<String> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "class_selector" => {
                    let name = child
                        .child_by_field_name("class_name")
                        .or_else(|| child.child(1))
                        .map(|n| node_text(n, src))?;
                    if !name.is_empty() {
                        return Some(format!(".{name}"));
                    }
                }
                "id_selector" => {
                    let name = child
                        .child_by_field_name("id_name")
                        .or_else(|| child.child(1))
                        .map(|n| node_text(n, src))?;
                    if !name.is_empty() {
                        return Some(format!("#{name}"));
                    }
                }
                "placeholder" => {
                    let name = child.child(1).map(|n| node_text(n, src))?;
                    if !name.is_empty() {
                        return Some(format!("%{name}"));
                    }
                }
                "tag_name" | "nesting_selector" | "universal_selector" => {
                    let t = node_text(child, src);
                    if !t.is_empty() {
                        return Some(t);
                    }
                }
                "pseudo_class_selector" | "pseudo_element_selector" => {
                    let t = node_text(child, src);
                    if !t.is_empty() && !t.contains('{') {
                        return Some(t);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

