// =============================================================================
// parser/extractors/python.rs  —  Python symbol and reference extractor
//
// What we extract
// ---------------
// SYMBOLS:
//   Class, Function, Method, Constructor, Property,
//   Variable (module-level assignments), Test
//
// REFERENCES:
//   - `import` / `from … import` → Import edges
//   - `call` expressions          → Calls edges
//   - Superclass lists            → TypeRef edges
//
// Approach:
//   Single-pass recursive CST walk. Qualified names are built by threading a
//   `qualified_prefix` string and an `inside_class` flag through the recursion.
//   Decorated definitions forward their decorator list to the inner function or
//   class extractor so `@property` and `@pytest.mark.*` are handled correctly.
//
// Visibility convention:
//   `__name` (dunder, no trailing `__`) → Private
//   `_name`                              → Private
//   everything else                      → Public
// =============================================================================

use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind,
    Visibility,
};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub struct PythonExtraction {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub has_errors: bool,
}

/// Extract all symbols and references from Python source code.
pub fn extract(source: &str) -> PythonExtraction {
    let language = tree_sitter_python::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Python grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return PythonExtraction {
                symbols: vec![],
                refs: vec![],
                has_errors: true,
            }
        }
    };

    let mut symbols = Vec::new();
    let mut refs = Vec::new();

    extract_from_node(
        tree.root_node(),
        source,
        &mut symbols,
        &mut refs,
        None,
        "",
        false,
    );

    let has_errors = tree.root_node().has_error();
    PythonExtraction { symbols, refs, has_errors }
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
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
                    &[],
                );
            }

            "class_definition" => {
                extract_class_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "decorated_definition" => {
                extract_decorated_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }

            "import_statement" => {
                extract_import_statement(&child, source, refs, symbols.len());
            }

            "import_from_statement" => {
                extract_import_from_statement(&child, source, refs, symbols.len());
            }

            "expression_statement" => {
                // Module-level or class-level assignments: CONST = value
                extract_assignment_if_any(
                    &child,
                    source,
                    symbols,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }

            // Skip tree-sitter error recovery nodes
            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Function definition
// ---------------------------------------------------------------------------

fn extract_function_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
    decorators: &[String],
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
    });

    if let Some(body_node) = body {
        extract_calls_from_body(&body_node, source, idx, refs);
    }
}

// ---------------------------------------------------------------------------
// Class definition
// ---------------------------------------------------------------------------

fn extract_class_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
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
    });

    // Extract superclass TypeRef edges from argument_list / superclasses field
    if let Some(superclasses) = node.child_by_field_name("superclasses") {
        extract_superclass_refs(&superclasses, source, refs, idx);
    }

    // Recurse into body — inside_class = true
    if let Some(body_node) = body {
        extract_from_node(
            body_node,
            source,
            symbols,
            refs,
            Some(idx),
            &new_prefix,
            true,
        );
    }
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
                refs.push(ExtractedRef {
                    source_symbol_index: class_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
            "attribute" => {
                // e.g. unittest.TestCase — extract the attribute name
                if let Some(attr) = child.child_by_field_name("attribute") {
                    let name = node_text(&attr, source);
                    let obj = child
                        .child_by_field_name("object")
                        .map(|o| node_text(&o, source));
                    refs.push(ExtractedRef {
                        source_symbol_index: class_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: obj,
                        chain: None,
                    });
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Decorated definitions
// ---------------------------------------------------------------------------

fn extract_decorated_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let decorators = extract_decorator_names(node, source);

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
                );
            }
            "class_definition" => {
                extract_class_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
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
                        // @pytest.mark.parametrize → "pytest.mark.parametrize"
                        names.push(node_text(&dchild, source));
                    }
                    "call" => {
                        // @pytest.mark.parametrize(...) → "pytest.mark.parametrize"
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

// ---------------------------------------------------------------------------
// Import statements
// ---------------------------------------------------------------------------

fn extract_import_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    // `import foo` or `import foo.bar` or `import foo, bar`
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                // `import foo.bar.baz` — last segment is target, rest is module
                let full = node_text(&child, source);
                let parts: Vec<&str> = full.split('.').collect();
                let target = parts.last().unwrap_or(&full.as_str()).to_string();
                let module = if parts.len() > 1 {
                    Some(parts[..parts.len() - 1].join("."))
                } else {
                    None
                };
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module,
                    chain: None,
                });
            }
            "aliased_import" => {
                // `import foo as f` — use original name
                if let Some(name_node) = child.child_by_field_name("name") {
                    let full = node_text(&name_node, source);
                    let parts: Vec<&str> = full.split('.').collect();
                    let target = parts.last().unwrap_or(&full.as_str()).to_string();
                    let module = if parts.len() > 1 {
                        Some(parts[..parts.len() - 1].join("."))
                    } else {
                        None
                    };
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module,
                        chain: None,
                    });
                }
            }
            _ => {}
        }
    }
}

fn extract_import_from_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    // `from foo.bar import baz, qux`
    let module = node.child_by_field_name("module_name").map(|m| {
        node_text(&m, source).trim_start_matches('.').to_string()
    });

    let module_name_node = node.child_by_field_name("module_name");

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip keywords and punctuation
        match child.kind() {
            "from" | "import" | "," | "import_prefix" => continue,
            _ => {}
        }
        // Skip the module_name node itself
        if let Some(ref mn) = module_name_node {
            if child.id() == mn.id() {
                continue;
            }
        }

        match child.kind() {
            "dotted_name" | "identifier" => {
                let name = node_text(&child, source);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: name,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: module.clone(),
                    chain: None,
                });
            }
            "aliased_import" => {
                // `from x import y as z` — use original name, not alias
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, source);
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: module.clone(),
                        chain: None,
                    });
                }
            }
            "wildcard_import" => {
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: "*".to_string(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: module.clone(),
                    chain: None,
                });
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Assignments (constants / variables)
// ---------------------------------------------------------------------------

fn extract_assignment_if_any(
    expr_stmt: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let mut cursor = expr_stmt.walk();
    for child in expr_stmt.children(&mut cursor) {
        if child.kind() == "assignment" {
            extract_assignment_node(
                &child,
                source,
                symbols,
                parent_index,
                qualified_prefix,
                inside_class,
            );
        }
        // augmented_assignment (+=, -=) and named_expression (:=) skipped intentionally
    }
}

fn extract_assignment_node(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let left = match node.child_by_field_name("left") {
        Some(n) => n,
        None => return,
    };

    match left.kind() {
        "identifier" => {
            let name = node_text(&left, source);
            let kind = classify_assignment_name(&name, inside_class);
            push_variable_symbol(
                node,
                &left,
                &name,
                kind,
                source,
                symbols,
                parent_index,
                qualified_prefix,
            );
        }
        "pattern_list" | "tuple_pattern" => {
            // `a, b = ...` — extract each identifier
            let mut cursor = left.walk();
            for elem in left.children(&mut cursor) {
                if elem.kind() == "identifier" {
                    let name = node_text(&elem, source);
                    let kind = classify_assignment_name(&name, inside_class);
                    push_variable_symbol(
                        node,
                        &elem,
                        &name,
                        kind,
                        source,
                        symbols,
                        parent_index,
                        qualified_prefix,
                    );
                }
            }
        }
        _ => {}
    }
}

fn classify_assignment_name(name: &str, inside_class: bool) -> SymbolKind {
    let stripped = name.trim_start_matches('_');
    if !inside_class
        && !stripped.is_empty()
        && stripped == stripped.to_uppercase()
        && stripped.chars().any(|c| c.is_alphabetic())
    {
        // v3 has no separate Constant kind — Variable covers both
        SymbolKind::Variable
    } else {
        SymbolKind::Variable
    }
}

#[allow(clippy::too_many_arguments)]
fn push_variable_symbol(
    node: &Node,
    name_node: &Node,
    name: &str,
    kind: SymbolKind,
    _source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let qualified_name = qualify(name, qualified_prefix);
    let visibility = detect_python_visibility(name);

    symbols.push(ExtractedSymbol {
        name: name.to_string(),
        qualified_name,
        kind,
        visibility,
        start_line: name_node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: name_node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

fn extract_calls_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let chain = build_chain(&func_node, source);
                let target_name = chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.name.clone())
                    .or_else(|| {
                        // Fallback: last segment of dotted text
                        let t = node_text(&func_node, source);
                        Some(t.rsplit('.').next().unwrap_or(&t).to_string())
                    });

                if let Some(target_name) = target_name {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name,
                        kind: EdgeKind::Calls,
                        line: func_node.start_position().row as u32,
                        module: None,
                        chain,
                    });
                }
            }
        }
        extract_calls_from_body(&child, source, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

/// Build a structured member access chain from a Python CST node.
///
/// Python uses `attribute` for member access and `call` for invocations:
///
/// ```text
/// call
///   attribute @function
///     attribute @object
///       identifier "self"
///       identifier "repo"
///     identifier "findOne"
///   argument_list
/// ```
/// produces: `[self, repo, findOne]`
fn build_chain(node: &Node, src: &str) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, src);
            let kind = if name == "self" || name == "cls" {
                SegmentKind::SelfRef
            } else {
                SegmentKind::Identifier
            };
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "attribute" => {
            // attribute { object: <expr>, attribute: identifier }
            let object = node.child_by_field_name("object")?;
            let attribute = node.child_by_field_name("attribute")?;

            build_chain_inner(&object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text(&attribute, src),
                node_kind: "attribute".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call" => {
            // Chained call: `a.b().c()` — the function child carries the chain.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(&func, src, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn node_text(node: &Node, source: &str) -> String {
    source[node.start_byte()..node.end_byte()].to_string()
}

fn qualify(name: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn scope_from_prefix(prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        None
    } else {
        Some(prefix.to_string())
    }
}

/// Python visibility convention:
///   `__name` (dunder without trailing `__`) → Private
///   `_name`                                  → Private
///   everything else                          → Public
fn detect_python_visibility(name: &str) -> Option<Visibility> {
    if name.starts_with("__") && !name.ends_with("__") {
        Some(Visibility::Private)
    } else if name.starts_with('_') {
        Some(Visibility::Private)
    } else {
        Some(Visibility::Public)
    }
}

/// Return the first docstring from a function/class body node.
/// A docstring is the first `expression_statement` containing a `string` literal.
fn extract_docstring(body: &Node, source: &str) -> Option<String> {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            let mut inner = child.walk();
            for expr in child.children(&mut inner) {
                if expr.kind() == "string" {
                    let raw = node_text(&expr, source);
                    let stripped = raw
                        .trim_start_matches("\"\"\"")
                        .trim_end_matches("\"\"\"")
                        .trim_start_matches("'''")
                        .trim_end_matches("'''")
                        .trim_start_matches('"')
                        .trim_end_matches('"')
                        .trim_start_matches('\'')
                        .trim_end_matches('\'')
                        .trim()
                        .to_string();
                    return Some(stripped);
                }
                if expr.kind() == "concatenated_string" {
                    return Some(node_text(&expr, source));
                }
            }
            // If first expression_statement is not a string, there is no docstring.
            break;
        }
        if child.kind() != "comment" {
            break;
        }
    }
    None
}

/// Build `def name(params)` or `def name(params) -> return_type` from a
/// `function_definition` node.
fn extract_function_signature(node: &Node, source: &str) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    let params_node = node.child_by_field_name("parameters")?;
    let name = node_text(&name_node, source);
    let params = node_text(&params_node, source);

    let sig = if let Some(ret) = node.child_by_field_name("return_type") {
        let ret_text = node_text(&ret, source);
        let ret_clean = ret_text.trim_start_matches("->").trim();
        format!("def {name}{params} -> {ret_clean}")
    } else {
        format!("def {name}{params}")
    };

    Some(sig)
}

fn is_test_function(name: &str, has_test_decorator: bool) -> bool {
    name.starts_with("test_") || name.starts_with("test") || has_test_decorator
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "python_tests.rs"]
mod tests;
