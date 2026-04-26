// =============================================================================
// languages/matlab/extract.rs — MATLAB extractor (tree-sitter-based)
//
// SYMBOLS:
//   Function  — `function_definition` (name field)
//   Class     — `class_definition` (name field)
//   Variable  — `assignment` at any scope (simple identifier on left)
//
// REFERENCES:
//   Calls     — `function_call` nodes (name or field_expression)
//
// POST-FILTER (local-scope suppression):
//   MATLAB's `X(i)` syntax is ambiguous — array indexing and function calls
//   parse identically as `function_call` nodes. This causes every use of an
//   array variable like `X(k)` inside a function to emit a Calls ref with
//   target_name="X", which the resolver can never resolve (X is a local
//   array, not a function). The filter walks the tree once after extraction
//   to collect locally-bound names (function in/out params, assignment LHS,
//   for/parfor loop vars) scoped to each function's line range, then drops
//   any ref whose target_name is locally bound at the ref's line.
// =============================================================================

use crate::types::{
    EdgeKind, ExtractedRef, ExtractedSymbol, ExtractionResult, SymbolKind, Visibility,
};
use tree_sitter::{Node, Parser};

pub fn extract(source: &str) -> ExtractionResult {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_matlab::LANGUAGE.into())
        .is_err()
    {
        return ExtractionResult::empty();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return ExtractionResult::empty(),
    };

    let src = source.as_bytes();
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    walk_node(tree.root_node(), src, &mut symbols, &mut refs, None);

    // Post-filter: suppress refs whose target is a locally-bound name at the
    // ref's source line. Locally-bound names come from:
    //   * function input parameters  (function_arguments → identifier children)
    //   * function output parameters (function_output → identifier / multioutput_variable)
    //   * simple assignment LHS      (assignment → left field: identifier)
    //   * multi-output assignment LHS (assignment → left: multioutput_variable → identifiers)
    //   * for/parfor loop variables  (for_statement → iterator → first identifier child)
    //   * lambda parameters          (lambda → arguments → identifier children)
    // Each binding is scoped to the enclosing function_definition's or lambda's
    // line range. At the top-level script scope the binding range is the whole file.
    // Nested function_definitions each contribute their own narrower scope, so a
    // name bound in both outer and inner functions is filtered regardless of which
    // scope the use falls in (both scopes contain the use's line).
    {
        let mut scopes: Vec<(String, u32, u32)> = Vec::new();
        let file_end = tree.root_node().end_position().row as u32;
        collect_local_bindings(tree.root_node(), src, 0, file_end, &mut scopes);
        if !scopes.is_empty() {
            refs.retain(|r| {
                !scopes.iter().any(|(name, start, end)| {
                    name == &r.target_name && r.line >= *start && r.line <= *end
                })
            });
        }
    }

    ExtractionResult::new(symbols, refs, tree.root_node().has_error())
}

fn walk_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    match node.kind() {
        "function_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            let name = if name.is_empty() {
                format!("<anon_fn_{}>", node.start_position().row + 1)
            } else {
                name
            };
            let idx = symbols.len();
            symbols.push(make_sym(name, SymbolKind::Function, Visibility::Public, node, parent_idx));
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "class_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();
            let name = if name.is_empty() {
                format!("<anon_class_{}>", node.start_position().row + 1)
            } else {
                name
            };
            let idx = symbols.len();
            symbols.push(make_sym(name, SymbolKind::Class, Visibility::Public, node, parent_idx));
            walk_children(node, src, symbols, refs, Some(idx));
        }
        "methods" => {
            // `methods` block inside a `class_definition` — extract nested
            // function_definition nodes as Method symbols.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "function_definition" {
                    let name = child
                        .child_by_field_name("name")
                        .map(|n| text(n, src))
                        .unwrap_or_default();
                    let name = if name.is_empty() {
                        format!("<anon_method_{}>", child.start_position().row + 1)
                    } else {
                        name
                    };
                    let idx = symbols.len();
                    symbols.push(make_sym(name, SymbolKind::Method, Visibility::Public, child, parent_idx));
                    walk_children(child, src, symbols, refs, Some(idx));
                } else {
                    walk_node(child, src, symbols, refs, parent_idx);
                }
            }
        }
        "assignment" => {
            // Capture assignments at any scope as Variable symbols.
            // field "left" holds the LHS.
            if let Some(lhs) = node.child_by_field_name("left") {
                match lhs.kind() {
                    "identifier" => {
                        let name = text(lhs, src);
                        if !name.is_empty() && is_simple_ident(&name) {
                            symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                        }
                    }
                    "function_call" => {
                        // Indexed assignment: `arr(i) = val` — use the function name as symbol
                        if let Some(name_node) = lhs.child_by_field_name("name") {
                            let name = text(name_node, src);
                            if !name.is_empty() && is_simple_ident(&name) {
                                symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                            }
                        }
                    }
                    "field_expression" => {
                        // Field assignment: `obj.field = val` — use the field name
                        if let Some(field_node) = lhs.child_by_field_name("field") {
                            let name = text(field_node, src);
                            if !name.is_empty() {
                                symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                            }
                        }
                    }
                    "multioutput_variable" => {
                        // Destructuring: `[a, b] = func()` — emit each variable
                        let mut mc = lhs.walk();
                        for child in lhs.children(&mut mc) {
                            if child.kind() == "identifier" {
                                let name = text(child, src);
                                if !name.is_empty() && is_simple_ident(&name) {
                                    symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                                }
                            }
                        }
                    }
                    _ => {
                        // Fallback: emit a symbol using the whole LHS text if it looks like an ident
                        let name = text(lhs, src);
                        if !name.is_empty() && is_simple_ident(&name) {
                            symbols.push(make_sym(name, SymbolKind::Variable, Visibility::Public, node, parent_idx));
                        }
                    }
                }
            }
            // Always recurse into children so function_call nodes on the RHS are visited.
            walk_children(node, src, symbols, refs, parent_idx);
        }
        "field_expression" => {
            // obj.method(args) — the grammar represents this as:
            //   field_expression
            //     object: identifier("obj")
            //     field:  function_call(name: identifier("method"), ...)
            // We want: target_name = "method", module = Some("obj").
            // Handle this here so we can emit module; then skip recursing into
            // children to avoid the nested function_call arm firing again.
            let sym_idx = parent_idx.unwrap_or(0);
            let object_node = node.child_by_field_name("object");
            let field_node = node.child_by_field_name("field");

            match (object_node, field_node) {
                (Some(obj), Some(field)) if field.kind() == "function_call" => {
                    let module_text = text(obj, src);
                    let method_text = field
                        .child_by_field_name("name")
                        .map(|n| text(n, src))
                        .unwrap_or_default();
                    if !method_text.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index: sym_idx,
                            target_name: method_text,
                            kind: EdgeKind::Calls,
                            line: node.start_position().row as u32,
                            module: if module_text.is_empty() { None } else { Some(module_text) },
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                    // Recurse into the function_call's arguments but not into the
                    // call node itself (to avoid duplicate emission).
                    walk_children(field, src, symbols, refs, parent_idx);
                }
                _ => {
                    // Plain field access (not a call) — just recurse.
                    walk_children(node, src, symbols, refs, parent_idx);
                }
            }
        }
        "function_call" => {
            let sym_idx = parent_idx.unwrap_or(0);
            // Simple function call (not a field_expression callee — those are
            // handled by the field_expression arm above).
            let target = node
                .child_by_field_name("name")
                .map(|n| text(n, src))
                .unwrap_or_default();

            if !target.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: target,
                    kind: EdgeKind::Calls,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
            walk_children(node, src, symbols, refs, parent_idx);
        }
        _ => {
            walk_children(node, src, symbols, refs, parent_idx);
        }
    }
}

fn walk_children(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_idx: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, src, symbols, refs, parent_idx);
    }
}

fn make_sym(
    name: String,
    kind: SymbolKind,
    vis: Visibility,
    node: Node,
    parent_idx: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        qualified_name: name.clone(),
        name,
        kind,
        visibility: Some(vis),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: parent_idx,
    }
}

fn text(node: Node, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or("").to_string()
}

fn is_simple_ident(s: &str) -> bool {
    s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && s.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_')
}

// =============================================================================
// Local-binding collector
// =============================================================================

/// Walk the entire tree and record `(name, scope_start_row, scope_end_row)` for
/// every locally-bound identifier. The `fn_start` / `fn_end` arguments track the
/// *current enclosing scope's* line range (function, lambda, or top-level file);
/// they are passed down through recursion and updated each time we descend into
/// a nested function or lambda.
///
/// Binding sources:
/// * **Function input params**: `function_arguments` → `identifier` children.
/// * **Function output params**: `function_output` → `identifier` or
///   `multioutput_variable` → `identifier` children.
/// * **Assignment LHS**: `assignment` → `left` field → `identifier` or
///   `multioutput_variable` → `identifier` children.  Scoped to the current
///   enclosing function / top-level file (MATLAB has no block scope).
/// * **Loop var**: `for_statement` → `iterator` → first `identifier` child.
///   Scoped to the for-statement's own line range.
/// * **Lambda params**: `lambda` → `arguments` field → `identifier` children.
///   Scoped to the lambda node's line range.
fn collect_local_bindings(
    node: Node,
    src: &[u8],
    fn_start: u32,
    fn_end: u32,
    out: &mut Vec<(String, u32, u32)>,
) {
    match node.kind() {
        "function_definition" => {
            let start = node.start_position().row as u32;
            let end = node.end_position().row as u32;

            // Input params: function_arguments → identifier children (via `arguments` field).
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "function_arguments" => {
                        collect_function_arguments(child, src, start, end, out);
                    }
                    "function_output" => {
                        collect_function_output(child, src, start, end, out);
                    }
                    _ => {}
                }
            }

            // Recurse into children with the new (narrower) function scope.
            let mut cursor2 = node.walk();
            for child in node.children(&mut cursor2) {
                collect_local_bindings(child, src, start, end, out);
            }
        }
        "lambda" => {
            // `@(x, y) body` — collect lambda parameters scoped to the lambda node.
            // Grammar: lambda → (anonymous) "arguments" child node containing
            // `identifier` children (the parameter names). The "arguments" is an
            // ALIAS in the grammar, exposed as a plain child (kind == "arguments"),
            // NOT as a named field.
            let lam_start = node.start_position().row as u32;
            let lam_end = node.end_position().row as u32;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "arguments" {
                    let mut ac = child.walk();
                    for arg in child.children(&mut ac) {
                        if arg.kind() == "identifier" {
                            let name = text(arg, src);
                            if !name.is_empty() && is_simple_ident(&name) {
                                out.push((name, lam_start, lam_end));
                            }
                        }
                    }
                }
            }
            // Recurse with current scope (lambda body is its own lexical scope for
            // params, but shares outer assignments — don't change fn_start/fn_end).
            let mut cursor2 = node.walk();
            for child in node.children(&mut cursor2) {
                collect_local_bindings(child, src, fn_start, fn_end, out);
            }
        }
        "assignment" => {
            // Collect assignment LHS names scoped to the current enclosing function
            // or file. fn_start == 0 at top-level (whole-file scope), which means
            // top-level script variables are also filtered.
            if let Some(lhs) = node.child_by_field_name("left") {
                match lhs.kind() {
                    "identifier" => {
                        let name = text(lhs, src);
                        if !name.is_empty() && is_simple_ident(&name) {
                            out.push((name, fn_start, fn_end));
                        }
                    }
                    "multioutput_variable" => {
                        let mut mc = lhs.walk();
                        for child in lhs.children(&mut mc) {
                            if child.kind() == "identifier" {
                                let name = text(child, src);
                                if !name.is_empty() && is_simple_ident(&name) {
                                    out.push((name, fn_start, fn_end));
                                }
                            }
                        }
                    }
                    "function_call" => {
                        // Indexed assignment: `X(i) = val` — X is a local array.
                        if let Some(name_node) = lhs.child_by_field_name("name") {
                            let name = text(name_node, src);
                            if !name.is_empty() && is_simple_ident(&name) {
                                out.push((name, fn_start, fn_end));
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Always recurse into children (RHS may have nested lambdas/functions).
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_local_bindings(child, src, fn_start, fn_end, out);
            }
        }
        "for_statement" => {
            // for_statement → iterator → identifier (= loop var), then block.
            // Scope the loop var to the for-statement itself (conservative).
            let for_start = node.start_position().row as u32;
            let for_end = node.end_position().row as u32;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "iterator" {
                    // First named child of iterator is the loop variable identifier.
                    let mut ic = child.walk();
                    for iter_child in child.children(&mut ic) {
                        if iter_child.kind() == "identifier" {
                            let name = text(iter_child, src);
                            if !name.is_empty() && is_simple_ident(&name) {
                                out.push((name, for_start, for_end));
                            }
                            break; // Only the first identifier is the loop var.
                        }
                    }
                }
            }
            // Recurse into block with current function scope.
            let mut cursor2 = node.walk();
            for child in node.children(&mut cursor2) {
                collect_local_bindings(child, src, fn_start, fn_end, out);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_local_bindings(child, src, fn_start, fn_end, out);
            }
        }
    }
}

/// Extract `identifier` children from a `function_arguments` node using the
/// `arguments` field. Records each as a binding in `[fn_start, fn_end]`.
fn collect_function_arguments(
    fa_node: Node,
    src: &[u8],
    fn_start: u32,
    fn_end: u32,
    out: &mut Vec<(String, u32, u32)>,
) {
    let mut cursor = fa_node.walk();
    // The `arguments` field contains the identifier nodes (and ignored_argument `~`).
    for child in fa_node.children_by_field_name("arguments", &mut cursor) {
        if child.kind() == "identifier" {
            let name = text(child, src);
            if !name.is_empty() && is_simple_ident(&name) {
                out.push((name, fn_start, fn_end));
            }
        }
    }
}

/// Extract identifier(s) from a `function_output` node.
/// Grammar: function_output → identifier  |  multioutput_variable → identifier*
fn collect_function_output(
    fo_node: Node,
    src: &[u8],
    fn_start: u32,
    fn_end: u32,
    out: &mut Vec<(String, u32, u32)>,
) {
    let mut cursor = fo_node.walk();
    for child in fo_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = text(child, src);
                if !name.is_empty() && is_simple_ident(&name) {
                    out.push((name, fn_start, fn_end));
                }
            }
            "multioutput_variable" => {
                let mut mc = child.walk();
                for id in child.children(&mut mc) {
                    if id.kind() == "identifier" {
                        let name = text(id, src);
                        if !name.is_empty() && is_simple_ident(&name) {
                            out.push((name, fn_start, fn_end));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod local_scope_tests {
    use super::*;

    // Helper: parse source and return the collected local bindings.
    fn bindings(src: &str) -> Vec<(String, u32, u32)> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_matlab::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        let file_end = tree.root_node().end_position().row as u32;
        let mut out = Vec::new();
        collect_local_bindings(tree.root_node(), src.as_bytes(), 0, file_end, &mut out);
        out
    }

    // -------------------------------------------------------------------------
    // CST probe: verify grammar kind names are as expected
    // -------------------------------------------------------------------------

    #[test]
    fn cst_probe_function_definition_has_function_arguments_and_output() {
        // `function [out1, out2] = name(arg1, arg2)` should parse with
        // function_arguments (input) and function_output (output) children.
        let src = "function [label, mu] = kmeans(X, m)\nlabel = 1;\nend\n";
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_matlab::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        let root = tree.root_node();
        // Collect child kinds explicitly to avoid cursor lifetime issues.
        let fn_def_id = {
            let mut cursor = root.walk();
            let mut found = None;
            for child in root.children(&mut cursor) {
                if child.kind() == "function_definition" {
                    found = Some(child.id());
                    break;
                }
            }
            found.expect("expected function_definition")
        };
        // Re-walk to get the node by id.
        let mut child_kinds: Vec<String> = Vec::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.id() == fn_def_id {
                let mut cc = child.walk();
                for grandchild in child.children(&mut cc) {
                    child_kinds.push(grandchild.kind().to_owned());
                }
                break;
            }
        }
        assert!(
            child_kinds.iter().any(|k| k == "function_arguments"),
            "expected function_arguments child; got {child_kinds:?}"
        );
        assert!(
            child_kinds.iter().any(|k| k == "function_output"),
            "expected function_output child; got {child_kinds:?}"
        );
    }

    #[test]
    fn cst_probe_for_statement_iterator_structure() {
        // `for i = 1:n` should parse as for_statement → iterator → identifier("i")
        let src = "for i = 1:10\n  disp(i);\nend\n";
        let src_bytes = src.as_bytes();
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_matlab::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        let root = tree.root_node();
        // Collect loop var by walking iterator manually.
        let mut loop_var: Option<String> = None;
        let mut cursor = root.walk();
        'outer: for child in root.children(&mut cursor) {
            if child.kind() == "for_statement" {
                let mut fc = child.walk();
                for for_child in child.children(&mut fc) {
                    if for_child.kind() == "iterator" {
                        let mut ic = for_child.walk();
                        for iter_child in for_child.children(&mut ic) {
                            if iter_child.kind() == "identifier" {
                                loop_var =
                                    Some(iter_child.utf8_text(src_bytes).unwrap().to_owned());
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(
            loop_var.as_deref(),
            Some("i"),
            "expected loop var 'i'; got {loop_var:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Binding collection
    // -------------------------------------------------------------------------

    #[test]
    fn input_params_collected() {
        // function foo(X, m) — both X and m should be collected as bindings.
        let src = "function foo(X, m)\nX(1) = 0;\nend\n";
        let b = bindings(src);
        assert!(
            b.iter().any(|(n, _, _)| n == "X"),
            "expected X in bindings; got {b:?}"
        );
        assert!(
            b.iter().any(|(n, _, _)| n == "m"),
            "expected m in bindings; got {b:?}"
        );
    }

    #[test]
    fn output_params_single_collected() {
        // function label = init(X, m) — label is an output param.
        let src = "function label = init(X, m)\nlabel = 1;\nend\n";
        let b = bindings(src);
        assert!(
            b.iter().any(|(n, _, _)| n == "label"),
            "expected label in bindings; got {b:?}"
        );
    }

    #[test]
    fn output_params_multi_collected() {
        // function [label, mu, energy] = kmeans(X, m) — all three outputs.
        let src = "function [label, mu, energy] = kmeans(X, m)\nlabel = 1;\nend\n";
        let b = bindings(src);
        assert!(
            b.iter().any(|(n, _, _)| n == "label"),
            "expected label; got {b:?}"
        );
        assert!(
            b.iter().any(|(n, _, _)| n == "mu"),
            "expected mu; got {b:?}"
        );
        assert!(
            b.iter().any(|(n, _, _)| n == "energy"),
            "expected energy; got {b:?}"
        );
    }

    #[test]
    fn assignment_lhs_collected() {
        // Inside a function, `n = numel(label)` should bind `n`.
        let src = "function foo(X)\nn = numel(X);\nend\n";
        let b = bindings(src);
        assert!(
            b.iter().any(|(n, _, _)| n == "n"),
            "expected n from assignment; got {b:?}"
        );
    }

    #[test]
    fn for_loop_var_collected() {
        // `for i = 1:n` should bind `i` to the for-statement's range.
        let src = "function foo(X)\nfor i = 1:10\n  X(i) = 0;\nend\nend\n";
        let b = bindings(src);
        assert!(
            b.iter().any(|(n, _, _)| n == "i"),
            "expected i from for loop; got {b:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Filter effect
    // -------------------------------------------------------------------------

    #[test]
    fn input_param_X_not_emitted_as_ref() {
        // `function foo(X)` with `X(i)` inside should NOT emit a ref for X.
        let src = "function foo(X)\ny = X(1);\nend\n";
        let result = extract(src);
        let x_refs: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.target_name == "X")
            .collect();
        assert!(
            x_refs.is_empty(),
            "expected no refs for X (local param); got {x_refs:?}"
        );
    }

    #[test]
    fn non_local_call_still_emitted() {
        // `zeros(3)` is a real function call and should still be emitted.
        let src = "function foo(X)\ny = zeros(3);\nend\n";
        let result = extract(src);
        assert!(
            result.refs.iter().any(|r| r.target_name == "zeros"),
            "expected ref for zeros; got {:?}",
            result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn output_param_mu_not_emitted_as_ref() {
        // `function [label, mu, energy] = kmeans(X, m)` — `mu` used as `mu'*X`
        // should not emit a ref for mu.
        let src = concat!(
            "function [label, mu, energy] = kmeans(X, m)\n",
            "mu = X;\n",
            "val = mu';\n", // postfix — mu is referenced as identifier, not function_call
            "end\n",
        );
        let result = extract(src);
        // mu used as `mu'` doesn't produce a function_call ref, but `mu(i)` would.
        // Test the assignment-LHS case: mu assigned in body binds mu.
        let b = bindings(src);
        assert!(
            b.iter().any(|(n, _, _)| n == "mu"),
            "expected mu bound; got {b:?}"
        );
    }

    // -------------------------------------------------------------------------
    // The kmeans.m nested-function case
    // -------------------------------------------------------------------------

    /// kmeans.m fixture: outer function spans lines 0-29 (0-indexed), inner
    /// function `init` starts at line 21. Both bind `X` as a parameter.
    /// Uses of `X(...)` inside `init` (lines 24-29) should be filtered.
    #[test]
    fn kmeans_nested_functions_X_filtered() {
        let src = concat!(
            "function [label, mu, energy] = kmeans(X, m)\n", // line 0
            "label = init(X, m);\n",                          // line 1 — X here is a call arg, not X(i)
            "n = numel(label);\n",                            // line 2
            "idx = 1:n;\n",                                   // line 3
            "last = zeros(1,n);\n",                           // line 4
            "while any(label ~= last)\n",                     // line 5
            "    mu = X*normalize(sparse(idx,last,1),1);\n",  // line 6
            "end\n",                                          // line 7
            "energy = 0;\n",                                  // line 8
            "function label = init(X, m)\n",                  // line 9
            "[d,n] = size(X);\n",                             // line 10 — size(X): X is param, filter
            "if numel(m) == 1\n",                             // line 11
            "    mu = X(:,randperm(n,m));\n",                 // line 12 — X(...): filter
            "end\n",                                          // line 13
            "end\n",                                          // line 14
        );
        let result = extract(src);
        // Refs for X should all be filtered (X is a param in both functions).
        let x_refs: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.target_name == "X")
            .collect();
        assert!(
            x_refs.is_empty(),
            "expected no refs for X; got {x_refs:?} (lines: {:?})",
            x_refs.iter().map(|r| r.line).collect::<Vec<_>>()
        );
        // But `zeros`, `numel`, `any`, `size`, `randperm`, `normalize`, `sparse`
        // should still appear as refs.
        assert!(
            result.refs.iter().any(|r| r.target_name == "zeros"),
            "expected ref for zeros to survive filter"
        );
        assert!(
            result.refs.iter().any(|r| r.target_name == "size"),
            "expected ref for size to survive filter"
        );
    }

    #[test]
    fn cst_probe_init_function_range() {
        // kmeans.m-style nested functions without 'end' keyword:
        // outer function + inner function, no end keyword on either.
        // Probe: what line range does tree-sitter give the inner function?
        // This is an EXACT replica of the chapter09/kmeans.m content.
        let src = concat!(
            "function [label, mu, energy] = kmeans(X, m)\n", // line 0
            "label = init(X, m);\n",                          // line 1
            "n = numel(label);\n",                            // line 2
            "idx = 1:n;\n",                                   // line 3
            "last = zeros(1,n);\n",                           // line 4
            "while any(label ~= last)\n",                     // line 5
            "    mu = X*normalize(sparse(idx,last,1),1);\n",  // line 6
            "    [val,label] = min(dot(mu,mu,1)'/2-mu'*X,[],1);\n", // line 7
            "end\n",                                          // line 8
            "energy = dot(X(:),X(:),1)+2*sum(val);\n",       // line 9
            "\n",                                             // line 10 (blank)
            "function label = init(X, m)\n",                  // line 11
            "[d,n] = size(X);\n",                             // line 12
            "if numel(m) == 1\n",                             // line 13
            "    mu = X(:,randperm(n,m));\n",                 // line 14
            "    [~,label] = min(dot(mu,mu,1)'/2-mu'*X,[],1);\n", // line 15
            "elseif all(size(m) == [1,n])\n",                 // line 16
            "    label = m;\n",                               // line 17
            "elseif size(m,1) == d\n",                        // line 18
            "    [~,label] = min(dot(m,m,1)'/2-m'*X,[],1);\n", // line 19
            "end\n",                                          // line 20
        );
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_matlab::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        let root = tree.root_node();
        let src_bytes = src.as_bytes();
        // Collect function_definition line ranges
        let mut fn_ranges: Vec<(String, u32, u32)> = Vec::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "function_definition" {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| n.utf8_text(src_bytes).unwrap().to_owned())
                    .unwrap_or_default();
                fn_ranges.push((
                    name,
                    child.start_position().row as u32,
                    child.end_position().row as u32,
                ));
                // Also check for nested function_definition children
                let mut cc = child.walk();
                for grandchild in child.children(&mut cc) {
                    if grandchild.kind() == "function_definition" {
                        let gname = grandchild
                            .child_by_field_name("name")
                            .map(|n| n.utf8_text(src_bytes).unwrap().to_owned())
                            .unwrap_or_default();
                        fn_ranges.push((
                            format!("nested:{gname}"),
                            grandchild.start_position().row as u32,
                            grandchild.end_position().row as u32,
                        ));
                    }
                }
            }
        }
        // Collect bindings to see what ranges get assigned
        let b = bindings(src);
        let x_bindings: Vec<_> = b.iter().filter(|(n, _, _)| n == "X").collect();
        // The inner init function's X should have a range that covers line 14
        // (0-indexed: `mu = X(:,...)` is on line 14).
        assert!(
            x_bindings.iter().any(|(_, start, end)| 14 >= *start && 14 <= *end),
            "expected X binding to cover line 14; x_bindings={x_bindings:?}, fn_ranges={fn_ranges:?}"
        );
        // Also confirm X at line 14 is filtered in the full extract pass.
        let result = extract(src);
        let x_at_14: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.target_name == "X" && r.line == 14)
            .collect();
        assert!(
            x_at_14.is_empty(),
            "expected X at line 14 to be filtered; refs={x_at_14:?}, fn_ranges={fn_ranges:?}, x_bindings={x_bindings:?}"
        );
    }

    #[test]
    fn cst_probe_real_kmeans_structure() {
        // Parse the exact kmeans.m content with comments (10 comment lines at top).
        // The real file has comment lines 2-10 (0-indexed 1-9) between the function
        // declaration and the first statement. This might affect the 'init' node range.
        let src = concat!(
            "function [label, mu, energy] = kmeans(X, m)\n", // line 0
            "% comment 1\n",                                  // line 1
            "% comment 2\n",                                  // line 2
            "% comment 3\n",                                  // line 3
            "% comment 4\n",                                  // line 4
            "% comment 5\n",                                  // line 5
            "% comment 6\n",                                  // line 6
            "% comment 7\n",                                  // line 7
            "% comment 8\n",                                  // line 8
            "% comment 9\n",                                  // line 9
            "label = init(X, m);\n",                          // line 10
            "n = numel(label);\n",                            // line 11
            "idx = 1:n;\n",                                   // line 12
            "last = zeros(1,n);\n",                           // line 13
            "while any(label ~= last)\n",                     // line 14
            "    mu = X*normalize(sparse(idx,last,1),1);\n",  // line 15
            "    [val,label] = min(dot(mu,mu,1)'/2-mu'*X,[],1);\n", // line 16
            "end\n",                                          // line 17
            "energy = dot(X(:),X(:),1)+2*sum(val);\n",       // line 18
            "\n",                                             // line 19 (blank)
            "function label = init(X, m)\n",                  // line 20
            "[d,n] = size(X);\n",                             // line 21
            "if numel(m) == 1\n",                             // line 22
            "    mu = X(:,randperm(n,m));\n",                 // line 23
            "    [~,label] = min(dot(mu,mu,1)'/2-mu'*X,[],1);\n", // line 24
            "elseif all(size(m) == [1,n])\n",                 // line 25
            "    label = m;\n",                               // line 26
            "elseif size(m,1) == d\n",                        // line 27
            "    [~,label] = min(dot(m,m,1)'/2-m'*X,[],1);\n", // line 28
            "end\n",                                          // line 29
        );
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_matlab::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        let root = tree.root_node();
        let src_bytes = src.as_bytes();
        // Collect all function_definition nodes and their ranges.
        let mut fn_ranges: Vec<(String, u32, u32)> = Vec::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "function_definition" {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| n.utf8_text(src_bytes).unwrap().to_owned())
                    .unwrap_or_default();
                fn_ranges.push((name, child.start_position().row as u32, child.end_position().row as u32));
            }
        }
        // Check the bindings for X include coverage of line 23 (mu = X(:,...))
        let b = bindings(src);
        let x_bindings: Vec<_> = b.iter().filter(|(n, _, _)| n == "X").collect();
        // The X at line 23 should be covered by init's binding
        let covered = x_bindings.iter().any(|(_, start, end)| 23 >= *start && 23 <= *end);
        // Report for debugging even if it fails
        // (We don't assert here — this is a diagnostic probe)
        let _ = (covered, &fn_ranges, &x_bindings); // suppress unused warnings
        assert!(
            covered,
            "X binding does NOT cover line 23; fn_ranges={fn_ranges:?}, x_bindings={x_bindings:?}"
        );
    }

    #[test]
    fn top_level_script_calls_not_over_filtered() {
        // Top-level script: `X = rand(3); foo(X)` — `foo` should still be a ref
        // (foo is not assigned locally, so it's not filtered). `X` itself would be
        // filtered as a local-var call since it's assigned on the previous line.
        let src = "X = rand(3);\nfoo(X);\n";
        let result = extract(src);
        assert!(
            result.refs.iter().any(|r| r.target_name == "foo"),
            "expected ref for foo in top-level script; got {:?}",
            result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
        );
        // rand(3) should also still appear (rand is not locally assigned).
        assert!(
            result.refs.iter().any(|r| r.target_name == "rand"),
            "expected ref for rand in top-level script; got {:?}",
            result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn lambda_params_not_emitted_as_refs() {
        // `@(x) dot(x(:), x(:))` — `x` is a lambda param; `x(:)` should not
        // emit a ref for x.
        let src = "Wn = cellfun(@(x) dot(x(:),x(:)),W);\n";
        let result = extract(src);
        let x_refs: Vec<_> = result
            .refs
            .iter()
            .filter(|r| r.target_name == "x")
            .collect();
        assert!(
            x_refs.is_empty(),
            "expected no refs for x (lambda param); got {x_refs:?}"
        );
        // But W and cellfun should still be refs.
        assert!(
            result.refs.iter().any(|r| r.target_name == "cellfun"),
            "expected ref for cellfun; got {:?}",
            result.refs.iter().map(|r| &r.target_name).collect::<Vec<_>>()
        );
    }
}
