// =============================================================================
// python/symbols.rs  —  Symbol extractors for Python
// =============================================================================

use super::assignments::{extract_assignment_if_any, extract_augmented_assignment};
use super::calls::{build_chain, extract_calls_from_body, extract_fstring_calls};
use super::helpers::{
    detect_python_visibility, extract_docstring, extract_function_signature,
    extract_python_type_name, is_test_function, node_text, qualify, scope_from_prefix,
};
use super::statements::{
    extract_comprehension, extract_except_clause, extract_for_statement,
    extract_match_statement, extract_named_expression, extract_raise_statement,
    extract_with_statement,
};
use super::types::extract_type_alias;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use std::collections::HashMap;
use tree_sitter::Node;
pub(super) fn extract_function_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
    decorators: &[String],
    import_map: &HashMap<String, String>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_python_visibility(&name);

    let has_property = decorators.iter().any(|d| d == "property");
    let has_test_decorator = decorators.iter().any(|d| {
        d.starts_with("pytest.mark") || d == "test" || d.starts_with("pytest.fixture")
    });

    let kind = if has_property {
        SymbolKind::Property
    } else if name == "__init__" {
        SymbolKind::Constructor
    } else if is_test_function(&name, has_test_decorator) {
        SymbolKind::Test
    } else if inside_class {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let body = node.child_by_field_name("body");
    let doc_comment = body.as_ref().and_then(|b| extract_docstring(b, source));
    let signature = extract_function_signature(node, source);

    let qualified_name_str = qualified_name.clone();
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    if let Some(params) = node.child_by_field_name("parameters") {
        extract_python_typed_params_as_symbols(
            &params,
            source,
            symbols,
            refs,
            Some(idx),
            &qualified_name_str,
        );
        // Also extract call refs from default argument expressions in the parameter
        // list (e.g. `def foo(x=bar(), y=list())`) so those call nodes are covered.
        extract_calls_from_body(&params, source, idx, refs, import_map);
    }

    if let Some(body_node) = body {
        extract_calls_from_body(&body_node, source, idx, refs, import_map);
        // Walk body for constructs that emit Variable symbols in addition to calls.
        extract_body_symbols(
            &body_node,
            source,
            symbols,
            refs,
            Some(idx),
            &qualified_name_str,
            idx,
            import_map,
        );
    }
}

/// Walk a function/method body to emit Variable symbols for constructs that
/// tree-sitter surfaces as sub-expressions rather than statements.
///
/// Covers:
///   - `with_statement` -> alias variable + chain TypeRef
///   - `match_statement` -> pattern capture variables + class TypeRefs
///   - `named_expression` (walrus `:=`) -> variable + chain TypeRef
///   - `list/dict/set_comprehension` / `generator_expression` -> loop variable
///   - `lambda` -> parameter variables
///
/// Note: call extraction is already handled by `extract_calls_from_body`; this
/// function only handles the symbol-emitting side.
pub(super) fn extract_body_symbols(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_idx: usize,
    import_map: &HashMap<String, String>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `x = 1` / `x: int = 1` inside a function or class body.
            "expression_statement" => {
                extract_assignment_if_any(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    false,
                );
                // Also recurse in case there are nested structures.
                extract_body_symbols(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }
            "with_statement" => {
                extract_with_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }
            "match_statement" => {
                extract_match_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }
            "named_expression" => {
                extract_named_expression(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }
            "list_comprehension"
            | "dictionary_comprehension"
            | "set_comprehension"
            | "generator_expression" => {
                extract_comprehension(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }
            "lambda" => {
                extract_lambda(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }
            "f_string" | "fstring" => {
                extract_fstring_calls(&child, source, enclosing_idx, refs, import_map);
            }

            // `async for item in stream:` — same shape as for_statement; recurse body.
            "async_for_statement" | "for_statement" => {
                extract_for_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }

            // `async with session() as s:` — same logic as with_statement.
            "async_with_statement" => {
                extract_with_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }

            // `global x` / `nonlocal y` — scope annotations; no symbols emitted,
            // but recurse so any calls inside unusual grammar structures are found.
            "global_statement" | "nonlocal_statement" => {}

            // `raise ValueError("msg")` — extract calls within + TypeRef for exception.
            "raise_statement" => {
                extract_raise_statement(&child, source, enclosing_idx, refs, import_map);
            }

            // `assert isinstance(x, Foo)` — extract calls within the test expression.
            "assert_statement" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs, import_map);
            }

            // `del obj.field` — extract member access.
            "delete_statement" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs, import_map);
            }

            // `x += 1` / `self.count += 1` — extract member access on left side.
            "augmented_assignment" => {
                extract_augmented_assignment(&child, source, enclosing_idx, refs, import_map);
            }

            // `type Point = tuple[int, int]` (Python 3.12+)
            "type_alias_statement" => {
                extract_type_alias(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }

            // `try: ... except ValueError as e:` — extract exception TypeRef and binding.
            "except_clause" | "except_group_clause" => {
                extract_except_clause(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }

            // `try: ... finally: ...` — recurse body.
            "try_statement" => {
                extract_body_symbols(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }

            // Nested `def` — extract as a symbol with the enclosing function as parent.
            "function_definition" => {
                extract_function_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    false, // nested def inside a function body is not a class method
                    &[],
                    import_map,
                );
            }

            // Nested `class` inside a function body.
            "class_definition" => {
                extract_class_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    import_map,
                    &[],
                );
            }

            // Nested decorated `def` or `class`.
            "decorated_definition" => {
                extract_decorated_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    false,
                    import_map,
                );
            }

            // `if __name__ == '__main__':` — recurse body to find any defs inside.
            "if_statement" => {
                extract_body_symbols(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }

            // `yield value` / `yield from iter` — recurse for calls.
            "yield" | "yield_statement" | "yield_expression" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs, import_map);
            }

            // Conditional expression: `a if cond else b` — recurse both branches.
            "conditional_expression" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs, import_map);
            }

            // Structural/container expressions — recurse for calls.
            "tuple" | "list" | "dictionary" | "set" | "slice"
            | "parenthesized_expression" | "starred_expression"
            | "binary_operator" | "boolean_operator"
            | "comparison_operator" | "unary_operator"
            | "not_operator" | "await" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs, import_map);
                // Also recurse for nested body-symbols (e.g. comprehensions inside tuples).
                extract_body_symbols(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }

            _ => {
                extract_body_symbols(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                    import_map,
                );
            }
        }
    }
}

pub(super) fn extract_python_typed_params_as_symbols(
    params_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    func_qualified_name: &str,
) {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        match child.kind() {
            "typed_parameter" | "typed_default_parameter" => {
                let (name, type_node) = if child.kind() == "typed_parameter" {
                    let type_node = match child.child_by_field_name("type") {
                        Some(t) => t,
                        None => continue,
                    };
                    let name_node = (0..child.child_count())
                        .filter_map(|i| child.child(i))
                        .find(|c| c.kind() == "identifier");
                    let name = match name_node {
                        Some(n) => node_text(&n, source),
                        None => continue,
                    };
                    (name, type_node)
                } else {
                    let name_node = match child.child_by_field_name("name") {
                        Some(n) => n,
                        None => continue,
                    };
                    let type_node = match child.child_by_field_name("type") {
                        Some(t) => t,
                        None => continue,
                    };
                    (node_text(&name_node, source), type_node)
                };

                if name == "self" || name == "cls" {
                    continue;
                }

                let type_name = extract_python_type_name(&type_node, source);
                if type_name.is_empty() {
                    continue;
                }

                let qualified_name = qualify(&name, func_qualified_name);
                let scope_path = Some(func_qualified_name.to_string());

                let param_idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name,
                    kind: SymbolKind::Parameter,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: Some(format!("{name}: {type_name}")),
                    doc_comment: None,
                    scope_path,
                    parent_index,
                                    byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index: param_idx,
                    target_name: type_name,
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: type_node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }

            // Untyped default: `def foo(x=5)`.
            "default_parameter" => {
                let name_node = match child.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };
                let name = node_text(&name_node, source);
                if name.is_empty() || name == "self" || name == "cls" {
                    continue;
                }
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: qualify(&name, func_qualified_name),
                    kind: SymbolKind::Parameter,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some(func_qualified_name.to_string()),
                    parent_index,
                                    byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
            }

            // `*args` — list splat parameter.
            "list_splat_pattern" => {
                // The identifier is the child of the splat node.
                let mut cc = child.walk();
                for c in child.children(&mut cc) {
                    if c.kind() == "identifier" {
                        let name = node_text(&c, source);
                        if !name.is_empty() && name != "self" && name != "cls" {
                            symbols.push(ExtractedSymbol {
                                name: name.clone(),
                                qualified_name: qualify(&name, func_qualified_name),
                                kind: SymbolKind::Parameter,
                                visibility: None,
                                start_line: c.start_position().row as u32,
                                end_line: c.end_position().row as u32,
                                start_col: c.start_position().column as u32,
                                end_col: c.end_position().column as u32,
                                signature: Some(format!("*{name}")),
                                doc_comment: None,
                                scope_path: Some(func_qualified_name.to_string()),
                                parent_index,
                                                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
                        }
                        break;
                    }
                }
            }

            // `**kwargs` — dictionary splat parameter.
            "dictionary_splat_pattern" => {
                let mut cc = child.walk();
                for c in child.children(&mut cc) {
                    if c.kind() == "identifier" {
                        let name = node_text(&c, source);
                        if !name.is_empty() && name != "self" && name != "cls" {
                            symbols.push(ExtractedSymbol {
                                name: name.clone(),
                                qualified_name: qualify(&name, func_qualified_name),
                                kind: SymbolKind::Parameter,
                                visibility: None,
                                start_line: c.start_position().row as u32,
                                end_line: c.end_position().row as u32,
                                start_col: c.start_position().column as u32,
                                end_col: c.end_position().column as u32,
                                signature: Some(format!("**{name}")),
                                doc_comment: None,
                                scope_path: Some(func_qualified_name.to_string()),
                                parent_index,
                                                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
                        }
                        break;
                    }
                }
            }

            _ => {}
        }
    }
}

pub(super) fn extract_class_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    import_map: &HashMap<String, String>,
    decorators: &[String],
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_python_visibility(&name);

    let body = node.child_by_field_name("body");
    let doc_comment = body.as_ref().and_then(|b| extract_docstring(b, source));

    let signature = {
        let text = node_text(node, source);
        text.lines()
            .next()
            .map(|l| l.trim_end_matches(':').trim().to_string())
    };

    let idx = symbols.len();

    let new_prefix = if qualified_prefix.is_empty() {
        name.clone()
    } else {
        format!("{}.{}", qualified_prefix, name)
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

    if let Some(superclasses) = node.child_by_field_name("superclasses") {
        extract_superclass_refs(&superclasses, source, refs, idx);
    }

    if let Some(body_node) = body {
        super::extract::extract_from_node(body_node, source, symbols, refs, Some(idx), &new_prefix, true, import_map);
    }

    if decorators.iter().any(is_dataclass_decorator) {
        synthesize_dataclass_init(node, source, symbols, idx, &new_prefix);
    }
}

/// Returns true when the decorator name is a recognised `@dataclass`
/// invocation. Recognises bare `dataclass`, qualified `dataclasses.dataclass`,
/// and re-exports that end in `.dataclass`.
fn is_dataclass_decorator(name: &String) -> bool {
    name == "dataclass" || name.ends_with(".dataclass")
}

/// Push a synthetic `__init__` Method symbol for a `@dataclass`-decorated
/// class. Param list mirrors the class body's annotated assignments in source
/// order. `ClassVar[...]` and dunder names are skipped (PEP 557). When the
/// source already declares `__init__` explicitly, no synthesis runs — the
/// real method wins.
fn synthesize_dataclass_init(
    class_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    class_idx: usize,
    class_qname: &str,
) {
    let body = match class_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    let explicit_init = symbols[class_idx + 1..].iter().any(|s| {
        s.name == "__init__"
            && s.scope_path.as_deref() == Some(class_qname)
    });
    if explicit_init {
        return;
    }

    let mut fields: Vec<(String, Option<String>)> = Vec::new();
    let mut body_cursor = body.walk();
    for stmt in body.children(&mut body_cursor) {
        if stmt.kind() != "expression_statement" {
            continue;
        }
        let mut stmt_cursor = stmt.walk();
        for inner in stmt.children(&mut stmt_cursor) {
            if inner.kind() != "assignment" {
                continue;
            }
            let Some(left) = inner.child_by_field_name("left") else {
                continue;
            };
            if left.kind() != "identifier" {
                continue;
            }
            let name = node_text(&left, source);
            if name.starts_with("__") {
                continue;
            }
            let annotation = inner
                .child_by_field_name("type")
                .map(|t| node_text(&t, source).trim().to_string());
            if annotation.is_none() {
                continue;
            }
            if let Some(ann) = &annotation {
                if ann.starts_with("ClassVar") {
                    continue;
                }
            }
            fields.push((name, annotation));
        }
    }

    if fields.is_empty() {
        return;
    }

    let mut sig = String::from("__init__(self");
    for (name, ann) in &fields {
        sig.push_str(", ");
        sig.push_str(name);
        if let Some(t) = ann {
            sig.push_str(": ");
            sig.push_str(t);
        }
    }
    sig.push(')');

    let qualified_name = format!("{class_qname}.__init__");
    let start_line = symbols[class_idx].start_line;
    let byte_offset = symbols[class_idx].byte_offset;

    symbols.push(ExtractedSymbol {
        name: "__init__".to_string(),
        qualified_name,
        kind: SymbolKind::Method,
        visibility: super::helpers::detect_python_visibility("__init__"),
        start_line,
        end_line: start_line,
        start_col: 0,
        end_col: 0,
        byte_offset,
        signature: Some(sig),
        doc_comment: None,
        scope_path: Some(class_qname.to_string()),
        parent_index: Some(class_idx),
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
}

fn extract_superclass_refs(
    argument_list: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    class_idx: usize,
) {
    let mut cursor = argument_list.walk();
    for child in argument_list.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(&child, source);
                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index: class_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            "attribute" => {
                if let Some(attr) = child.child_by_field_name("attribute") {
                    let name = node_text(&attr, source);
                    let obj = child
                        .child_by_field_name("object")
                        .map(|o| node_text(&o, source));
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index: class_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: obj,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            _ => {}
        }
    }
}

pub(super) fn extract_decorated_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
    import_map: &HashMap<String, String>,
) {
    let decorators = extract_decorator_names(node, source);
    let symbol_index = symbols.len();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                extract_function_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    &decorators,
                    import_map,
                );
                super::decorators::extract_decorators(node, source, symbol_index, refs);
            }
            "class_definition" => {
                extract_class_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    import_map,
                    &decorators,
                );
                super::decorators::extract_decorators(node, source, symbol_index, refs);
            }
            _ => {}
        }
    }
}

fn extract_decorator_names(node: &Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let mut dcursor = child.walk();
            for dchild in child.children(&mut dcursor) {
                match dchild.kind() {
                    "identifier" => {
                        names.push(node_text(&dchild, source));
                    }
                    "attribute" => {
                        names.push(node_text(&dchild, source));
                    }
                    "call" => {
                        if let Some(func) = dchild.child_by_field_name("function") {
                            names.push(node_text(&func, source));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    names
}


pub(super) fn extract_lambda(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
    import_map: &HashMap<String, String>,
) {
    if let Some(params) = node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for param in params.children(&mut cursor) {
            let name = match param.kind() {
                "identifier" => node_text(&param, source),
                "default_parameter" | "typed_parameter" | "typed_default_parameter" => param
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default(),
                _ => continue,
            };
            if name.is_empty() || name == "self" || name == "cls" {
                continue;
            }
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: qualify(&name, qualified_prefix),
                kind: SymbolKind::Parameter,
                visibility: detect_python_visibility(&name),
                start_line: param.start_position().row as u32,
                end_line: param.end_position().row as u32,
                start_col: param.start_position().column as u32,
                end_col: param.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
        }
    }

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, source, enclosing_symbol_index, refs, import_map);
    }
}

