// =============================================================================
// swift/calls.rs  —  Call extraction and member chain builder for Swift
// =============================================================================

use super::helpers::{call_target_name, node_text};
use super::predicates;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                if let Some(callee) = child.named_child(0) {
                    let chain = build_chain(callee, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| call_target_name(&callee, src));
                    let call_args = extract_swift_call_args(&child, src);
                    crate::languages::emit_chain_type_ref(
                        &chain,
                        source_symbol_index,
                        &callee,
                        refs,
                    );
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain,
                            byte_offset: callee.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `expr is Type` — emit TypeRef for the checked type.
            "check_expression" => {
                let named: Vec<_> = {
                    let mut nc = child.walk();
                    child.named_children(&mut nc).collect()
                };
                // check_expression: [expression, check_operator, type]
                // The last named child is the type.
                if let Some(type_node) = named.last() {
                    let kind = type_node.kind();
                    if kind != "is_operator" && kind != "is" && kind != "check_operator" {
                        extract_type_ref_from_swift_type(type_node, src, source_symbol_index, refs);
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `expr as Type` — emit TypeRef for the cast type.
            // In tree-sitter-swift the type is the last named child (after as_operator).
            "as_expression" => {
                // Walk named children: skip the lhs expression and as_operator; the
                // remaining named child is the type node.
                let mut nc = child.walk();
                let named: Vec<_> = child.named_children(&mut nc).collect();
                if let Some(type_node) = named.last() {
                    if type_node.kind() != "as_operator" {
                        extract_type_ref_from_swift_type(type_node, src, source_symbol_index, refs);
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `{ params in body }` — recurse into lambda/closure body.
            "lambda_literal" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `"\(expr)"` — recurse into interpolated expressions.
            "line_string_literal" | "multi_line_string_literal" => {
                let mut sc = child.walk();
                for seg in child.children(&mut sc) {
                    if seg.kind() == "interpolated_expression" {
                        extract_calls_from_body(&seg, src, source_symbol_index, refs);
                    }
                }
            }

            // Type references that appear anywhere inside function/closure bodies:
            //   - local variable type annotations  (`let x: MyType = ...`)
            //   - explicit type casts              (`x as! MyType`)
            //   - generic argument lists           (`Array<MyType>`)
            //   - inheritance specifiers on nested types
            "user_type" | "optional_type" | "metatype_type" => {
                extract_type_ref_from_swift_type(&child, src, source_symbol_index, refs);
                // Recurse so generic type arguments inside user_type also emit refs.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `SomeProtocol & AnotherProtocol` — emit a ref per component.
            "protocol_composition_type" => {
                extract_protocol_composition_refs(&child, src, source_symbol_index, refs);
            }

            // `var x: MyType` or `let x: MyType` — type annotation inside body.
            "type_annotation" => {
                if let Some(type_node) = child
                    .child_by_field_name("type")
                    .or_else(|| child.named_child(0))
                {
                    if type_node.kind() == "protocol_composition_type" {
                        extract_protocol_composition_refs(
                            &type_node,
                            src,
                            source_symbol_index,
                            refs,
                        );
                    } else {
                        extract_type_ref_from_swift_type(
                            &type_node,
                            src,
                            source_symbol_index,
                            refs,
                        );
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `inheritance_specifier` in nested class/struct bodies.
            "inheritance_specifier" | "type_inheritance_clause" => {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    match inner.kind() {
                        "user_type" | "type_identifier" => {
                            extract_type_ref_from_swift_type(
                                &inner,
                                src,
                                source_symbol_index,
                                refs,
                            );
                        }
                        _ => {}
                    }
                }
            }

            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Emit a TypeRef for a Swift type node (user_type, optional_type, array_type, etc.).
pub(super) fn extract_type_ref_from_swift_type(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = swift_type_name(node, src);
    if !name.is_empty() {
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
    // Recursively walk the entire type subtree to catch ALL type_identifier nodes,
    // including in generic arguments and nested type expressions.
    extract_all_type_identifiers(node, src, source_symbol_index, refs);
}

/// Walk a type node recursively and emit TypeRef for every type_identifier found.
/// This ensures comprehensive coverage of generic parameters, nested types, etc.
/// Public version for use in extract.rs.
pub(super) fn extract_all_type_identifiers_from_node(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_all_type_identifiers(node, src, source_symbol_index, refs);
}

fn extract_all_type_identifiers(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Only `type_identifier` is a type; `simple_identifier` is a value
            // identifier (property, argument label, closure `$0`) and must not
            // be emitted as a TypeRef. It falls to the recurse arm (a leaf, so
            // a no-op) rather than polluting the type graph.
            "type_identifier" => {
                let name = node_text(child, src);
                let line = child.start_position().row as u32;
                // Filter language-primitive type names to keep refs honest.
                if !name.is_empty() && !predicates::is_swift_primitive_type(&name) {
                    // Deduplicate only if same name AND same line — different lines
                    // need separate refs so coverage correlation matches by line.
                    let already_emitted = refs.iter().rev().take(5).any(|r| {
                        r.source_symbol_index == source_symbol_index
                            && r.target_name == name
                            && r.line == line
                            && r.kind == EdgeKind::TypeRef
                    });
                    if !already_emitted {
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                            col: 0,
                        });
                    }
                }
            }
            _ => {
                // Recurse into children to find nested type_identifier nodes.
                extract_all_type_identifiers(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract the simple name from a Swift type node.
pub(super) fn swift_type_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "user_type" => {
            // user_type → type_identifier+ (e.g. `Array` or `Swift.Array`).
            // Take the last type_identifier.
            let mut last = String::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    // Only `type_identifier` names a type. A `user_type` that
                    // wraps a `simple_identifier` is a value expression the
                    // grammar mis-grouped (closure param, local) — emitting its
                    // text as a TypeRef pollutes the type graph.
                    "type_identifier" => {
                        last = node_text(child, src);
                    }
                    _ => {}
                }
            }
            last
        }
        "optional_type" => {
            // Recurse into the wrapped type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let n = swift_type_name(&child, src);
                if !n.is_empty() {
                    return n;
                }
            }
            String::new()
        }
        "array_type" => {
            // `[T]` — the element type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let n = swift_type_name(&child, src);
                if !n.is_empty() {
                    return n;
                }
            }
            String::new()
        }
        "dictionary_type" => {
            // `[K: V]` — just emit the key type.
            if let Some(key) = node.named_child(0) {
                return swift_type_name(&key, src);
            }
            String::new()
        }
        "function_type" => {
            // `(A) -> B` — emit return type.
            if let Some(ret) = node.child_by_field_name("return_type") {
                return swift_type_name(&ret, src);
            }
            String::new()
        }
        // `simple_identifier` / `identifier` are value identifiers (closure
        // params, locals); only `type_identifier` denotes a Swift type.
        "type_identifier" => node_text(*node, src),
        // `SomeProtocol & AnotherProtocol` — emit the first component
        "protocol_composition_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let n = swift_type_name(&child, src);
                if !n.is_empty() {
                    return n;
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Emit TypeRef edges for ALL type components in a protocol_composition_type.
pub(super) fn extract_protocol_composition_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() != "protocol_composition_type" {
        extract_type_ref_from_swift_type(node, src, source_symbol_index, refs);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let n = swift_type_name(&child, src);
        if !n.is_empty() {
            refs.push(ExtractedRef {
                is_include: false,
                is_import_binding: false,
                is_reexport: false,
                source_symbol_index,
                target_name: n,
                kind: crate::types::EdgeKind::TypeRef,
                line: child.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: child.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
        }
    }
}

/// Build a structured member-access chain from a Swift call expression's callee node.
pub(super) fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    match node.kind() {
        "simple_identifier" | "identifier" => return None,
        _ => {}
    }
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "simple_identifier" | "identifier" | "type_identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "self_expression" => {
            segments.push(ChainSegment {
                name: "self".to_string(),
                node_kind: "self_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "super_expression" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "navigation_expression" => {
            let target = node.child_by_field_name("target")?;
            build_chain_inner(target, src, segments)?;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "navigation_suffix" {
                    let mut nc = child.walk();
                    for inner in child.children(&mut nc) {
                        if inner.kind() == "simple_identifier" {
                            segments.push(ChainSegment {
                                name: node_text(inner, src),
                                node_kind: "simple_identifier".to_string(),
                                kind: SegmentKind::Property,
                                declared_type: None,
                                type_args: vec![],
                                optional_chaining: false,
                                byte_offset: 0,
                                declared_type_id: None,
                                is_call: false,
                                call_args: Vec::new(),
                                type_arg_ids: Vec::new(),
                            });
                            return Some(());
                        }
                    }
                }
            }
            None
        }

        "call_expression" => {
            // Mark the invoked segment so the walker yields the function's return
            // type rather than the function value (closure call-yield).
            let callee = node.named_child(0)?;
            build_chain_inner(callee, src, segments)?;
            if let Some(last) = segments.last_mut() {
                last.is_call = true;
            }
            Some(())
        }

        _ => None,
    }
}

pub(super) fn extract_swift_call_args(call_node: &Node, src: &[u8]) -> Vec<crate::types::CallArg> {
    use crate::types::CallArg;
    let mut args_node: Option<Node> = None;
    let mut cursor = call_node.walk();
    for c in call_node.children(&mut cursor) {
        if c.kind() == "call_suffix" || c.kind() == "value_arguments" {
            args_node = Some(c);
            break;
        }
    }
    let Some(args) = args_node else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut ac = args.walk();
    for child in args.named_children(&mut ac) {
        let inner = if child.kind() == "value_argument" {
            let mut vc = child.walk();
            child.named_children(&mut vc).last().unwrap_or(child)
        } else {
            child
        };
        let arg = match inner.kind() {
            "line_string_literal" | "raw_string_literal" | "string_literal" => {
                let raw = node_text(inner, src);
                CallArg::StringLit(raw.trim_matches('"').to_string())
            }
            "simple_identifier" | "identifier" => CallArg::Ident(node_text(inner, src)),
            "integer_literal" | "real_literal" => CallArg::Literal(node_text(inner, src)),
            "boolean_literal" | "nil_literal" | "nil" => CallArg::Literal(inner.kind().to_string()),
            // `{ x in x.foo }` — named closure parameter — or the anonymous-
            // shorthand form `{ $0.foo }`. Capture the closure's parameter names
            // (named, or synthetic `$0..$max`) so the chain walker can type them
            // from the higher-order method's callback-parameter signature.
            "lambda_literal" => CallArg::Lambda {
                params: swift_closure_param_names(&inner, src),
            },
            _ => CallArg::Other,
        };
        out.push(arg);
    }
    out
}

/// Collect the positional parameter names of a Swift `lambda_literal`.
///
/// Named parameters live under a `type` field as a `lambda_function_type`
/// whose `lambda_function_type_parameters` hold `lambda_parameter` nodes with a
/// `name` field (`{ x in x.foo }` → `["x"]`). When the closure declares no
/// named parameters, the anonymous-shorthand form (`{ $0.foo }`, `{ $0 + $1 }`)
/// is recognized: `\$[0-9]+` is a `simple_identifier` alternative in the Swift
/// grammar, so each `$N` is a real token in the closure body. The dense list
/// `["$0".."$max"]` is returned, where `max` is the highest index referenced.
/// The seeding step then types each `$N` from the callback signature exactly as
/// for a named param; an unreferenced dense slot is a harmless no-op seed.
fn swift_closure_param_names(node: &Node, src: &[u8]) -> Vec<String> {
    if let Some(ty) = node.child_by_field_name("type") {
        if let Some(params) = swift_named_child(&ty, "lambda_function_type_parameters") {
            let mut cursor = params.walk();
            return params
                .named_children(&mut cursor)
                .filter(|p| p.kind() == "lambda_parameter")
                .map(|p| {
                    p.child_by_field_name("name")
                        .map(|n| node_text(n, src))
                        .unwrap_or_default()
                })
                .collect();
        }
    }
    // No named parameter list — recognize the anonymous-shorthand form.
    match swift_max_shorthand_index(node, src) {
        Some(max) => (0..=max).map(|i| format!("${i}")).collect(),
        None => Vec::new(),
    }
}

/// Highest `$N` index referenced in this closure's own body, or `None` when the
/// closure uses no shorthand parameters. Descent stops at nested `lambda_literal`
/// boundaries so an inner closure's `$N` never leaks to the outer closure's
/// parameter list (sound scoping).
fn swift_max_shorthand_index(node: &Node, src: &[u8]) -> Option<u32> {
    let mut max: Option<u32> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "lambda_literal" {
            continue;
        }
        if child.kind() == "simple_identifier" {
            if let Some(idx) = parse_shorthand_index(&node_text(child, src)) {
                max = Some(max.map_or(idx, |m| m.max(idx)));
            }
        }
        if let Some(inner) = swift_max_shorthand_index(&child, src) {
            max = Some(max.map_or(inner, |m| m.max(inner)));
        }
    }
    max
}

/// Parse a `$N` shorthand token into its index. Returns `None` for any other
/// identifier (`$foo`, `x`, named-capture `$`-prefixed bindings).
fn parse_shorthand_index(text: &str) -> Option<u32> {
    let digits = text.strip_prefix('$')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// First named child of `node` whose kind is `kind`, searched by index so the
/// returned node carries the tree lifetime rather than a local cursor's.
fn swift_named_child<'a>(node: &Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut i = 0;
    while let Some(child) = node.named_child(i) {
        if child.kind() == kind {
            return Some(child);
        }
        i += 1;
    }
    None
}
