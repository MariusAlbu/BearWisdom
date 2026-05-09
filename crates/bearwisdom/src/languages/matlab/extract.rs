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
                    // Cell-array indexing like `obj.lu{mm}` parses with a `{`-bearing
                    // name — these are not callable targets.
                    if !method_text.is_empty()
                        && !method_text.contains('{')
                        && !method_text.contains('}')
                    {
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
            if let Some(name_node) = node.child_by_field_name("name") {
                let target = text(name_node, src);

                // Cell-array indexing like `Population{2}` leaks through as a
                // function_call whose name contains `{` or `}`.
                let has_brace = target.contains('{') || target.contains('}');

                // tree-sitter-matlab's ERROR recovery for `...` line-continuation
                // can truncate 1–2 leading bytes of the next identifier. When the
                // byte immediately before this name node's start is alphabetic or
                // `_`, the name is the trailing fragment of a longer token.
                let start_byte = name_node.start_byte();
                let is_truncated = start_byte > 0
                    && src
                        .get(start_byte - 1)
                        .map_or(false, |&b| b.is_ascii_alphabetic() || b == b'_');

                if !target.is_empty() && !has_brace && !is_truncated {
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
/// * **Indexed struct-field assignment LHS**: `obj.foo.bar(idx) = ...` — the
///   trailing field name (the last non-call segment) is bound to suppress the
///   phantom call ref the extractor would otherwise emit for `bar`.
pub(crate) fn collect_local_bindings(
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
                    "field_expression" => {
                        // Indexed struct-field assignment: `obj.app.dropD(1) = val`.
                        // The grammar nests this as a chain of field_expression nodes
                        // where the innermost field is either an identifier (plain
                        // field) or a function_call (indexed field). Bind the trailing
                        // segment name so the phantom call ref is suppressed.
                        if let Some(name) = trailing_field_name(lhs, src) {
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

/// Walk a `field_expression` chain and return the trailing segment name.
///
/// For `obj.app.dropD(1)` the tree is:
///   field_expression(object=field_expression(object=obj, field=app), field=function_call(name=dropD))
/// Returns `"dropD"` — the name of the innermost `field` child, unwrapping one
/// level of `function_call` if the field is an indexed access.
fn trailing_field_name(fe_node: Node, src: &[u8]) -> Option<String> {
    let field = fe_node.child_by_field_name("field")?;
    match field.kind() {
        "identifier" => Some(text(field, src)),
        "function_call" => field
            .child_by_field_name("name")
            .map(|n| text(n, src)),
        "field_expression" => trailing_field_name(field, src),
        _ => None,
    }
}

/// Extract `identifier` children from a `function_arguments` node using the
/// `arguments` field. Records each as a binding in `[fn_start, fn_end]`.
pub(crate) fn collect_function_arguments(
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
pub(crate) fn collect_function_output(
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
#[path = "extract_tests.rs"]
mod local_scope_tests;
