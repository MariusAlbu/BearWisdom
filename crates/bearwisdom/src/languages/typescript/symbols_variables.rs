use super::calls::build_chain;
use super::helpers::{detect_visibility, node_text};
use super::symbols_casts::{
    extract_type_ref_from_as_expression, extract_type_ref_from_satisfies_expression,
    extract_type_ref_from_type_assertion,
};
use super::types::extract_type_ref_from_annotation;
use crate::parser::scope_tree;
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, SegmentKind, SymbolKind,
};
use tree_sitter::Node;

pub(super) fn push_variable_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_tree::scope_path(parent_scope);

    // `const Foo = ...` — get declarators.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                // Capture simple identifiers and object destructuring patterns.
                if name_node.kind() == "identifier" {
                    let name = node_text(name_node, src);
                    let qualified_name = scope_tree::qualify(&name, parent_scope);
                    let idx = symbols.len();
                    // Promote to Function when the initializer is a function-like expression.
                    // `const f = function() {}` or `const f = () => {}` -- standard TS idiom
                    // for top-level functions written as variable assignments.
                    let init_kind = child.child_by_field_name("value").map(|v| v.kind());
                    let sym_kind = match init_kind.as_deref() {
                        Some("arrow_function" | "function_expression") => SymbolKind::Function,
                        _ => SymbolKind::Variable,
                    };
                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name,
                        kind: sym_kind,
                        visibility: detect_visibility(node, src),
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: Some(format!("const {name}")),
                        doc_comment: None,
                        scope_path: scope_path.clone(),
                        parent_index,
                                            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

                    // Extract TypeRef from variable type annotation: `const repo: Repository`
                    if let Some(type_ann) = child.child_by_field_name("type") {
                        extract_type_ref_from_annotation(&type_ann, src, idx, refs);
                    } else if let Some(init) = child.child_by_field_name("value") {
                        // No explicit type — try to infer from initializer.
                        // `const user = this.repo.findOne(1)` → chain [this, repo, findOne]
                        // Emit a chain-bearing TypeRef so the index builder can
                        // resolve the chain's return type as the variable's type.
                        let init_node = if init.kind() == "await_expression" {
                            // `const user = await this.repo.findOne(1)` → unwrap await
                            init.child_by_field_name("value")
                                .or_else(|| init.named_child(0))
                                .unwrap_or(init)
                        } else {
                            init
                        };
                        if init_node.kind() == "call_expression" {
                            if let Some(func) = init_node.child_by_field_name("function") {
                                if let Some(chain) = build_chain(func, src) {
                                    // Use the last segment as the target_name.
                                    let target = chain
                                        .segments
                                        .last()
                                        .map(|s| s.name.clone())
                                        .unwrap_or_default();
                                    if !target.is_empty() {
                                        refs.push(ExtractedRef {
                                            source_symbol_index: idx,
                                            target_name: target,
                                            kind: EdgeKind::TypeRef,
                                            line: init_node.start_position().row as u32,
                                            col: 0,
                                            module: None,
                                            chain: Some(chain),
                                            byte_offset: init_node.start_byte() as u32,
                                                                                    namespace_segments: Vec::new(),
                                                                                    call_args: Vec::new(),
});
                                    }
                                }
                            }
                        } else if init_node.kind() == "new_expression" {
                            // `const map = new Map()` → type is the constructor name
                            if let Some(constructor) = init_node.child_by_field_name("constructor") {
                                let type_name = match constructor.kind() {
                                    "identifier" | "type_identifier" => {
                                        node_text(constructor, src)
                                    }
                                    _ => String::new(),
                                };
                                if !type_name.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: idx,
                                        target_name: type_name,
                                        kind: EdgeKind::TypeRef,
                                        line: init_node.start_position().row as u32,
                                        col: 0,
                                        module: None,
                                        chain: None,
                                        byte_offset: constructor.start_byte() as u32,
                                                                            namespace_segments: Vec::new(),
                                                                            call_args: Vec::new(),
});
                                }
                            }
                        } else if init_node.kind() == "member_expression" {
                            // `const x = obj.field` → chain for field type inference
                            if let Some(chain) = build_chain(init_node, src) {
                                let target = chain
                                    .segments
                                    .last()
                                    .map(|s| s.name.clone())
                                    .unwrap_or_default();
                                if !target.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: idx,
                                        target_name: target,
                                        kind: EdgeKind::TypeRef,
                                        line: init_node.start_position().row as u32,
                                        col: 0,
                                        module: None,
                                        chain: Some(chain),
                                        byte_offset: init_node.start_byte() as u32,
                                                                            namespace_segments: Vec::new(),
                                                                            call_args: Vec::new(),
});
                                }
                            }
                        } else if init_node.kind() == "as_expression" {
                            // `const admin = user as Admin` → type is Admin
                            // The type node is the last named child after the `as` keyword.
                            extract_type_ref_from_as_expression(&init_node, src, idx, refs);
                        } else if init_node.kind() == "type_assertion" {
                            // `const admin = <Admin>user` → type is Admin
                            // type_assertion has type_arguments as first child.
                            extract_type_ref_from_type_assertion(&init_node, src, idx, refs);
                        } else if init_node.kind() == "satisfies_expression" {
                            // `const obj = { a: 1 } satisfies Config` → type is Config
                            // satisfies_expression: <expr> satisfies <type>
                            // The type is the last named child after the `satisfies` keyword.
                            extract_type_ref_from_satisfies_expression(
                                &init_node, src, idx, refs,
                            );
                        }
                    }
                } else if name_node.kind() == "object_pattern" {
                    // const { name, email } = user
                    // Extract each destructured property as a Variable symbol
                    // with a TypeRef chain to the source expression.
                    let source_chain = child
                        .child_by_field_name("value")
                        .and_then(|init| build_chain(init, src));

                    let mut ppcursor = name_node.walk();
                    for prop in name_node.children(&mut ppcursor) {
                        // `binding_name` is what other code references — the local
                        // identifier introduced by the pattern. For
                        // `{ mutate: mutate1 }` the binding is `mutate1`, not
                        // `mutate`. Without this distinction, callers writing
                        // `mutate1()` look up a non-existent `mutate` symbol and
                        // the call goes unresolved.
                        //
                        // `source_prop` is the property name on the right-hand
                        // side, used to extend the type-resolution chain so the
                        // binding's inferred type matches the destructured field.
                        let (binding_name, source_prop) = if prop.kind()
                            == "shorthand_property_identifier_pattern"
                            || prop.kind() == "shorthand_property_identifier"
                        {
                            // `{ mutate }` — same name on both sides.
                            let n = node_text(prop, src);
                            (n.clone(), n)
                        } else if prop.kind() == "pair_pattern" {
                            let key = prop
                                .child_by_field_name("key")
                                .map(|k| node_text(k, src))
                                .unwrap_or_default();
                            let value_node = prop.child_by_field_name("value");
                            // The value can itself be another pattern
                            // (`{ a: { b } }`) or an assignment_pattern with a
                            // default (`{ a: b = 1 }`). Only emit a binding
                            // symbol when the value is a plain identifier — the
                            // nested cases are handled by the recursive walk.
                            let binding = match value_node.map(|v| (v.kind(), v)) {
                                Some(("identifier", v)) => node_text(v, src),
                                Some(("assignment_pattern", v)) => v
                                    .child_by_field_name("left")
                                    .filter(|l| l.kind() == "identifier")
                                    .map(|l| node_text(l, src))
                                    .unwrap_or_default(),
                                _ => String::new(),
                            };
                            if binding.is_empty() {
                                continue;
                            }
                            (binding, key)
                        } else {
                            continue;
                        };
                        if binding_name.is_empty() {
                            continue;
                        }

                        let qualified_name = scope_tree::qualify(&binding_name, parent_scope);
                        let prop_idx = symbols.len();
                        symbols.push(ExtractedSymbol {
                            name: binding_name.clone(),
                            qualified_name,
                            kind: SymbolKind::Variable,
                            visibility: detect_visibility(node, src),
                            start_line: prop.start_position().row as u32,
                            end_line: prop.end_position().row as u32,
                            start_col: prop.start_position().column as u32,
                            end_col: prop.end_position().column as u32,
                            signature: None,
                            doc_comment: None,
                            scope_path: scope_path.clone(),
                            parent_index,
                                                    byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

                        // Emit chain to source with property name appended so the
                        // index builder can resolve the type of this property.
                        if let Some(ref base_chain) = source_chain {
                            let mut prop_chain = base_chain.clone();
                            prop_chain.segments.push(ChainSegment {
                                name: source_prop.clone(),
                                node_kind: "property".to_string(),
                                kind: SegmentKind::Property,
                                declared_type: None,
                                type_args: vec![],
                                optional_chaining: false,
                                byte_offset: 0,
                                                            declared_type_id: None,
                                is_call: false,
                                type_arg_ids: Vec::new(),
});
                            refs.push(ExtractedRef {
                                source_symbol_index: prop_idx,
                                target_name: source_prop,
                                kind: EdgeKind::TypeRef,
                                line: prop.start_position().row as u32,
                                col: 0,
                                module: None,
                                chain: Some(prop_chain),
                                byte_offset: prop.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
                            });
                        }
                    }
                } else if name_node.kind() == "array_pattern" {
                    // `const [isOpen, setIsOpen] = useState(false)` —
                    // array destructuring. Each named element becomes a
                    // Variable symbol so references from the surrounding
                    // scope can resolve against it.
                    //
                    // For type inference, each destructured element gets a
                    // chain-bearing TypeRef: the initializer chain with an
                    // extra ComputedAccess segment whose `name` is the
                    // integer element index. The TypeInfo builder's
                    // infer_type_from_chain handles the tuple-index case —
                    // when the current type is a tuple literal like
                    // `[T, Dispatch<SetStateAction<T>>]`, the ComputedAccess
                    // segment selects the Nth element.
                    //
                    // Elision holes (`const [, setX] = ...`) are allowed in
                    // TS — the grammar exposes them as unnamed children and
                    // we increment the index so subsequent bindings still
                    // map to the right slot. Nested destructuring
                    // (`[[a, b], c]`) is ignored in MVP — the element isn't
                    // a plain identifier so no symbol is pushed.
                    let source_chain = child
                        .child_by_field_name("value")
                        .and_then(|init| build_chain(init, src));

                    let mut apcursor = name_node.walk();
                    let mut elem_index: usize = 0;
                    for element in name_node.children(&mut apcursor) {
                        // Punctuation like `[`, `,`, `]` has no meaningful
                        // role here — skip via `is_named()`. Elision holes
                        // aren't represented as a specific grammar node and
                        // are effectively already skipped because no named
                        // child sits between commas.
                        if !element.is_named() {
                            continue;
                        }
                        let is_rest = element.kind() == "rest_pattern";
                        let (elem_name, elem_node) = match element.kind() {
                            "identifier" => (node_text(element, src), element),
                            "assignment_pattern" => {
                                let Some(inner) = element.child_by_field_name("left") else {
                                    elem_index += 1;
                                    continue;
                                };
                                if inner.kind() != "identifier" {
                                    elem_index += 1;
                                    continue;
                                }
                                (node_text(inner, src), inner)
                            }
                            "rest_pattern" => {
                                let mut rc = element.walk();
                                let inner = element
                                    .children(&mut rc)
                                    .find(|c| c.kind() == "identifier");
                                let Some(inner) = inner else {
                                    elem_index += 1;
                                    continue;
                                };
                                (node_text(inner, src), inner)
                            }
                            _ => {
                                elem_index += 1;
                                continue;
                            }
                        };
                        if elem_name.is_empty() {
                            elem_index += 1;
                            continue;
                        }

                        let qualified_name = scope_tree::qualify(&elem_name, parent_scope);
                        let elem_sym_idx = symbols.len();
                        symbols.push(ExtractedSymbol {
                            name: elem_name.clone(),
                            qualified_name,
                            kind: SymbolKind::Variable,
                            visibility: detect_visibility(node, src),
                            start_line: elem_node.start_position().row as u32,
                            end_line: elem_node.end_position().row as u32,
                            start_col: elem_node.start_position().column as u32,
                            end_col: elem_node.end_position().column as u32,
                            signature: None,
                            doc_comment: None,
                            scope_path: scope_path.clone(),
                            parent_index,
                                                    byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});

                        // Rest elements (`...rest`) bind an array of the
                        // remaining tuple elements — tuple-index inference
                        // doesn't apply. Skip the TypeRef emission so the
                        // chain walker isn't misled into thinking it's the
                        // single element at `elem_index`.
                        if !is_rest {
                            if let Some(ref base_chain) = source_chain {
                                let mut elem_chain = base_chain.clone();
                                // The final segment carries the binding name so that
                                // the chain's last segment matches target_name (contract
                                // REF-004). The tuple index is stored in node_kind for
                                // type-inference consumers that need the positional slot.
                                elem_chain.segments.push(ChainSegment {
                                    name: elem_name.clone(),
                                    node_kind: format!("tuple_index:{}", elem_index),
                                    kind: SegmentKind::ComputedAccess,
                                    declared_type: None,
                                    type_args: vec![],
                                    optional_chaining: false,
                                    byte_offset: 0,
                                                                    declared_type_id: None,
                                    is_call: false,
                                    type_arg_ids: Vec::new(),
});
                                refs.push(ExtractedRef {
                                    source_symbol_index: elem_sym_idx,
                                    target_name: elem_name,
                                    kind: EdgeKind::TypeRef,
                                    line: elem_node.start_position().row as u32,
                                    col: 0,
                                    module: None,
                                    chain: Some(elem_chain),
                                    byte_offset: elem_node.start_byte() as u32,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                        }

                        elem_index += 1;
                    }
                }
            }
        }
    }
}
