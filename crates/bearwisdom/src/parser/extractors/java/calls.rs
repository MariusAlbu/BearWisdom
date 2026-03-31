// =============================================================================
// java/calls.rs  —  Call extraction and member chain building for Java
// =============================================================================

use super::helpers::{node_text, type_node_simple_name};
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind};
use tree_sitter::Node;

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_calls_from_body_with_symbols(node, src, source_symbol_index, refs, None);
}

/// Recursive call extractor that also emits Variable symbols for lambda
/// parameters when a `symbols` vec is provided.
pub(super) fn extract_calls_from_body_with_symbols(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    mut symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_invocation" => {
                // `name` field is always present (identifier).
                if let Some(name_node) = child.child_by_field_name("name") {
                    let chain = build_chain(&child, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| node_text(name_node, src));
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: name_node.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                // Recurse into arguments — nested calls.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            "object_creation_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = type_node_simple_name(type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            "lambda_expression" => {
                // Emit Variable symbols for lambda parameters, then recurse into the body.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_lambda_params(&child, src, source_symbol_index, syms);
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            // `if (user instanceof Admin admin)` — Java 16+ pattern matching instanceof.
            // Also handles older `if (user instanceof Admin)` without a pattern variable.
            "instanceof_expression" => {
                extract_instanceof_refs(&child, src, source_symbol_index, refs, symbols.as_deref_mut());
                // Don't recurse further — the instanceof_expression has no nested bodies.
            }

            // `catch (IOException | SQLException e)` — emit TypeRef for the exception type(s)
            // and a Variable symbol for the catch binding.
            "catch_clause" => {
                extract_catch_clause_refs(&child, src, source_symbol_index, refs, symbols.as_deref_mut());
                // Recurse into the catch body for nested calls.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `try (Resource r = new Resource())` — emit TypeRef and Variable for each resource.
            "try_with_resources_statement" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_try_with_resources_refs(&child, src, source_symbol_index, refs, Some(syms));
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_try_with_resources_refs(&child, src, source_symbol_index, refs, None);
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `(Type) value` — emit TypeRef for the cast target.
            "cast_expression" => {
                extract_cast_expression_refs(&child, src, source_symbol_index, refs);
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `Class::method` or `obj::method` — emit a Calls edge.
            "method_reference" => {
                extract_method_reference_calls(&child, src, source_symbol_index, refs);
                // No further recursion needed — method_reference has no sub-expressions.
            }

            // `for (var item : list)` — emit Variable for the loop variable,
            // TypeRef for an explicit declared type.
            "enhanced_for_statement" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_enhanced_for_refs(&child, src, source_symbol_index, refs, Some(syms));
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_enhanced_for_refs(&child, src, source_symbol_index, refs, None);
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `Foo.class` — emit TypeRef for the class name.
            "class_literal" => {
                extract_class_literal_ref(&child, src, source_symbol_index, refs);
            }

            _ => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// catch_clause helpers
// ---------------------------------------------------------------------------

/// Emit TypeRef(s) for the exception type(s) in a catch clause, plus a
/// Variable symbol for the catch parameter binding.
///
/// Tree-sitter-java shape:
/// ```text
/// catch_clause
///   catch_formal_parameter
///     catch_type              ← type_identifier | union_type
///     identifier              ← the binding variable
/// ```
fn extract_catch_clause_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "catch_formal_parameter" {
            continue;
        }

        let mut binding_name: Option<(String, Node)> = None;
        let mut cc = child.walk();
        for param_child in child.children(&mut cc) {
            match param_child.kind() {
                "catch_type" => {
                    // `catch_type` may contain multiple `type_identifier` children
                    // separated by `|` (multi-catch).
                    let mut tc = param_child.walk();
                    for type_node in param_child.children(&mut tc) {
                        let name = super::helpers::type_node_simple_name(type_node, src);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: type_node.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                }
                "identifier" => {
                    let name = node_text(param_child, src);
                    if !name.is_empty() {
                        binding_name = Some((name, param_child));
                    }
                }
                _ => {}
            }
        }

        if let (Some((name, binding_node)), Some(syms)) = (binding_name, symbols) {
            syms.push(make_variable_symbol(name, &binding_node, source_symbol_index));
            // Only one catch_formal_parameter per catch clause.
            break;
        }
        break;
    }
}

// ---------------------------------------------------------------------------
// try-with-resources helpers
// ---------------------------------------------------------------------------

fn extract_try_with_resources_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    mut symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "resource_specification" {
            let mut rc = child.walk();
            for res in child.children(&mut rc) {
                if res.kind() != "resource" {
                    continue;
                }
                // resource: type identifier = expression
                let type_node = res.child_by_field_name("type");
                let name_node = res.child_by_field_name("name")
                    .or_else(|| res.child_by_field_name("name"));

                if let Some(tn) = type_node {
                    let type_name = super::helpers::type_node_simple_name(tn, src);
                    if !type_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: type_name,
                            kind: EdgeKind::TypeRef,
                            line: tn.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }

                if let (Some(nn), Some(syms)) = (name_node, symbols.as_deref_mut()) {
                    let name = node_text(nn, src);
                    if !name.is_empty() {
                        syms.push(make_variable_symbol(name, &nn, source_symbol_index));
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// cast_expression helper
// ---------------------------------------------------------------------------

fn extract_cast_expression_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // cast_expression { type: _type, value: expression }
    if let Some(type_node) = node.child_by_field_name("type") {
        let name = super::helpers::type_node_simple_name(type_node, src);
        if !name.is_empty() && !super::helpers::is_java_primitive(&name) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// method_reference helper
// ---------------------------------------------------------------------------

/// Emit a Calls edge for `Class::method` or `obj::method`.
///
/// Tree-sitter-java shape:
/// ```text
/// method_reference
///   identifier | type_identifier | method_invocation  ← object/type
///   identifier                                        ← method name
/// ```
fn extract_method_reference_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The method name is the last identifier after `::`.
    // In tree-sitter-java the `::` is an anonymous token, and the method name
    // is the last named child or a child named `name`.
    let method_name = node.child_by_field_name("name")
        .map(|n| node_text(n, src))
        .or_else(|| {
            // Fall back: scan all named children and take the last identifier.
            let mut last: Option<String> = None;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    last = Some(node_text(child, src));
                }
            }
            last
        });

    if let Some(name) = method_name {
        if !name.is_empty() && name != "new" {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }

    // Also emit TypeRef for the receiver type if it's a type reference.
    // `String::valueOf` — the first child before `::` is `String`.
    let receiver = node.child_by_field_name("object")
        .or_else(|| {
            let children: Vec<_> = {
                let mut cursor = node.walk();
                node.children(&mut cursor).collect()
            };
            children.into_iter().find(|c| c.kind() == "type_identifier" || c.kind() == "identifier")
        });

    if let Some(recv) = receiver {
        if recv.kind() == "type_identifier" {
            let name = node_text(recv, src);
            if !name.is_empty() && !super::helpers::is_java_primitive(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: recv.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// enhanced_for_statement helper
// ---------------------------------------------------------------------------

fn extract_enhanced_for_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    // enhanced_for_statement { type: _type, name: identifier, value: expression, body: statement }
    let type_node = node.child_by_field_name("type");
    let name_node = node.child_by_field_name("name");

    if let Some(tn) = type_node {
        let type_name = super::helpers::type_node_simple_name(tn, src);
        if !type_name.is_empty() && !super::helpers::is_java_primitive(&type_name) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: tn.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }

    if let (Some(nn), Some(syms)) = (name_node, symbols) {
        let name = node_text(nn, src);
        if !name.is_empty() {
            syms.push(make_variable_symbol(name, &nn, source_symbol_index));
        }
    }
}

// ---------------------------------------------------------------------------
// class_literal helper
// ---------------------------------------------------------------------------

/// Emit a TypeRef for `Foo.class`.
///
/// Tree-sitter-java shape:
/// ```text
/// class_literal
///   type_identifier | generic_type | ...  ← the class type
///   "." "class"
/// ```
fn extract_class_literal_ref(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The type is the first named child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        let name = super::helpers::type_node_simple_name(child, src);
        if !name.is_empty() && !super::helpers::is_java_primitive(&name) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::TypeRef,
                line: child.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        break;
    }
}

/// Emit a TypeRef for `user instanceof Admin` and optionally a Variable symbol
/// for the pattern variable in `user instanceof Admin admin` (Java 16+).
///
/// Tree-sitter-java structure for `instanceof_expression`:
/// ```text
/// instanceof_expression
///   identifier "user"          ← left operand (field: "left")
///   type_identifier "Admin"    ← type (field: "right" or positional)
///   identifier "admin"         ← optional pattern variable
/// ```
fn extract_instanceof_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    // The type is the "right" field; fall back to scanning for type_identifier.
    let type_node = node.child_by_field_name("right").or_else(|| {
        let mut cursor = node.walk();
        let found = node
            .children(&mut cursor)
            .find(|c| c.kind() == "type_identifier" || c.kind() == "generic_type");
        found
    });

    let type_node = match type_node {
        Some(n) => n,
        None => return,
    };

    let type_name = match type_node.kind() {
        "type_identifier" => node_text(type_node, src),
        "generic_type" => {
            // `instanceof List<String>` — emit base type
            type_node
                .child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| node_text(type_node, src))
        }
        _ => node_text(type_node, src),
    };

    if type_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: type_name.clone(),
        kind: EdgeKind::TypeRef,
        line: type_node.start_position().row as u32,
        module: None,
        chain: None,
    });

    // Pattern variable: the identifier after the type, if present.
    // `user instanceof Admin admin` → emit Variable symbol `admin` with TypeRef to Admin.
    if let Some(syms) = symbols {
        // Find an identifier that comes after the type node.
        let type_end = type_node.end_byte();
        let mut cursor = node.walk();
        for c in node.children(&mut cursor) {
            if c.kind() == "identifier" && c.start_byte() > type_end {
                let var_name = node_text(c, src);
                if !var_name.is_empty() {
                    let var_idx = syms.len();
                    syms.push(make_variable_symbol(var_name, &c, source_symbol_index));
                    refs.push(ExtractedRef {
                        source_symbol_index: var_idx,
                        target_name: type_name.clone(),
                        kind: EdgeKind::TypeRef,
                        line: c.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                break;
            }
        }
    }
}

/// Extract parameters from a `lambda_expression` node and emit them as
/// Variable symbols.
///
/// Java lambda parameters can be:
/// - A bare `identifier`  → `x -> ...`
/// - `formal_parameters`  → `(String x, int y) -> ...`
/// - `inferred_parameters` → `(x, y) -> ...`
fn extract_lambda_params(
    lambda_node: &Node,
    src: &[u8],
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = lambda_node.walk();
    for child in lambda_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                // Single untyped parameter: `x -> ...`
                let name = node_text(child, src);
                if !name.is_empty() {
                    symbols.push(make_variable_symbol(name, &child, parent_index));
                }
                // Only the first identifier before `->` is the parameter.
                break;
            }
            "inferred_parameters" => {
                // `(x, y) -> ...` — bare identifier list.
                let mut ic = child.walk();
                for param in child.children(&mut ic) {
                    if param.kind() == "identifier" {
                        let name = node_text(param, src);
                        if !name.is_empty() {
                            symbols.push(make_variable_symbol(name, &param, parent_index));
                        }
                    }
                }
            }
            "formal_parameters" => {
                // `(Type x, Type y) -> ...`
                let mut fp = child.walk();
                for param in child.children(&mut fp) {
                    if param.kind() == "formal_parameter" {
                        if let Some(name_node) = param.child_by_field_name("name") {
                            let name = node_text(name_node, src);
                            if !name.is_empty() {
                                symbols.push(make_variable_symbol(name, &name_node, parent_index));
                            }
                        }
                    }
                }
            }
            "->" => break,
            _ => {}
        }
    }
}

fn make_variable_symbol(name: String, node: &Node, parent_index: usize) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

/// Build a structured member access chain from a Java CST node.
///
/// Java uses `method_invocation` for calls and `field_access` for member reads:
///
/// ```text
/// method_invocation
///   object: method_invocation      ← chained call
///     object: identifier "this"
///     name:   identifier "getRepo"
///   name: identifier "findOne"
/// ```
/// produces: `[this, getRepo, findOne]`
pub(super) fn build_chain(node: &Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            let name = node_text(*node, src);
            let kind = SegmentKind::Identifier;
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

        "this" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "super" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "method_invocation" => {
            // method_invocation { object?: <expr>, name: identifier }
            if let Some(object) = node.child_by_field_name("object") {
                build_chain_inner(&object, src, segments)?;
            }
            let name_node = node.child_by_field_name("name")?;
            segments.push(ChainSegment {
                name: node_text(name_node, src),
                node_kind: "method_invocation".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "field_access" => {
            // field_access { object: <expr>, field: identifier }
            let object = node.child_by_field_name("object")?;
            let field = node.child_by_field_name("field")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(field, src),
                node_kind: "field_access".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        _ => None,
    }
}
