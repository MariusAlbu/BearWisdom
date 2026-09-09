use super::*;

fn capture(source: &str) -> crate::indexer::lexical::LexicalBindings {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(
        !tree.root_node().has_error(),
        "{}",
        tree.root_node().to_sexp()
    );
    crate::indexer::lexical::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap()
}

#[test]
fn rowless_signature_types_retain_generic_owners_and_constraints() {
    let source = "interface Catalog<T> { <U extends T = T>(item: U): U; new<U extends T>(item: U): Catalog<U>; }";
    let graph = capture(source);
    let sigs = &graph.types.signatures;
    assert_eq!(sigs.len(), 3);
    assert!(sigs.iter().all(|s| s.declaration.is_none()));
    for signature in &sigs[1..] {
        assert!(
            matches!(signature.parameters[0], TypeExpr::SignatureParameter { owner, index: 0 } if owner == signature.id)
        );
        assert!(
            matches!(signature.generics[0].constraint, Some(TypeExpr::SignatureParameter { owner, index: 0 }) if owner == sigs[0].id)
        );
    }
    assert!(
        matches!(sigs[1].generics[0].default, Some(TypeExpr::SignatureParameter { owner, index: 0 }) if owner == sigs[0].id)
    );
    assert!(
        matches!(sigs[1].result, Some(TypeExpr::SignatureParameter { owner, index: 0 }) if owner == sigs[1].id)
    );
    assert!(matches!(sigs[2].result, Some(TypeExpr::Apply(_, _))));
}

#[test]
fn nested_function_type_binder_does_not_borrow_its_enclosing_parameter() {
    let graph = capture("interface Catalog<T> { visit<U>(callback: <U>(item: U) => T): U; }");
    let sigs = &graph.types.signatures;
    assert_eq!(sigs.len(), 3);
    assert!(
        matches!(sigs[1].result, Some(TypeExpr::SignatureParameter { owner, index: 0 }) if owner == sigs[1].id)
    );
    assert!(
        matches!(sigs[2].parameters[0], TypeExpr::SignatureParameter { owner, index: 0 } if owner == sigs[2].id)
    );
    assert!(
        matches!(sigs[2].result, Some(TypeExpr::SignatureParameter { owner, index: 0 }) if owner == sigs[0].id)
    );
}

#[test]
fn compiler_labelled_type_parameter_uses_match_source_owner_and_index() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../resolution_oracle/signature_type_fixtures.json"
    ))
    .unwrap();
    let markers = regex::Regex::new(r"/\*@(ref|decl):(\d+)\*/").unwrap();
    for case in cases {
        let source = case["source"].as_str().unwrap();
        let graph = capture(source);
        let mut refs = std::collections::HashMap::new();
        let mut declarations = std::collections::HashMap::new();
        for marker in markers.captures_iter(source) {
            let map = if &marker[1] == "ref" {
                &mut refs
            } else {
                &mut declarations
            };
            assert!(map
                .insert(
                    marker[2].parse::<u64>().unwrap(),
                    marker.get(0).unwrap().end() as u32
                )
                .is_none());
        }
        for label in case["labels"].as_array().unwrap() {
            let byte = refs[&label[0].as_u64().unwrap()];
            let name = &source[byte as usize..byte as usize + 1];
            let actual = graph
                .name_id(name)
                .and_then(|name| graph.type_binding_at(byte, name))
                .and_then(|binding| {
                    let owner = graph.type_parameter_sites.get(&binding)?;
                    let &(_, _, index) = graph.type_parameters.get(&binding)?;
                    Some(
                        graph
                            .types
                            .signatures
                            .iter()
                            .find(|s| &s.id == owner)?
                            .syntax
                            .type_parameters[index]
                            .start,
                    )
                });
            assert_eq!(
                actual,
                label[1].as_u64().map(|id| declarations[&id]),
                "{} reference {}",
                case["name"],
                label[0]
            );
        }
    }
}

#[test]
fn predefined_and_literal_source_types_are_not_legacy_name_payloads() {
    let graph = capture("interface Values { a: string; b: number; c: boolean; d: unknown; e: any; f: null; g: undefined; h: void; i: never; j: symbol; k: bigint; l: object; m: 'hello'; n: 1; o: true; }");
    assert_eq!(graph.types.signatures.len(), 15);
    for signature in &graph.types.signatures {
        assert!(
            !matches!(
                signature.result,
                Some(TypeExpr::Legacy(_) | TypeExpr::Unknown) | None
            ),
            "source atomic type lost its identity: {:?}",
            signature.result
        );
    }
}

#[test]
fn source_type_operators_are_not_legacy_name_payloads() {
    let graph = capture("interface Ops<T, K extends keyof T> { keys: keyof T; read: T[K]; frozen: readonly [T, K]; choice: T extends K ? T : K; }");
    assert_eq!(graph.types.signatures.len(), 5);
    for signature in &graph.types.signatures[1..] {
        assert!(
            !matches!(
                signature.result,
                Some(TypeExpr::Legacy(_) | TypeExpr::Unknown) | None
            ),
            "source type operator lost its identity: {:?}",
            signature.result
        );
    }
}

#[test]
fn unique_symbol_property_annotations_are_not_unresolved_atomic_types() {
    let graph = capture(
        "interface Keys { readonly first: unique symbol; readonly second: unique symbol; }",
    );
    for signature in &graph.types.signatures {
        assert!(signature.syntax.unique_symbol);
        assert!(
            !matches!(
                signature.result,
                Some(TypeExpr::Unknown | TypeExpr::Legacy(_)) | None
            ),
            "{:?}",
            signature.result
        );
    }
}

#[test]
fn compiler_labelled_operator_shapes_keep_exact_operand_generic_owners() {
    use crate::type_checker::core::types::{LitValue, TypeOperator as Op};
    use serde_json::{json, Value};
    fn shape(
        expr: &TypeExpr,
        owner: SignatureId,
        graph: &crate::indexer::lexical::LexicalBindings,
    ) -> Value {
        let child = |expr| shape(expr, owner, graph);
        match expr {
            TypeExpr::SignatureParameter {
                owner: actual,
                index,
            } => {
                assert_eq!(*actual, owner);
                json!(["param", index])
            }
            TypeExpr::Operator(op) => match op.as_ref() {
                Op::KeyOf(t) => json!(["keyof", child(t)]),
                Op::Readonly(t) => json!(["readonly", child(t)]),
                Op::IndexedAccess { object, index } => {
                    json!(["index", child(object), child(index)])
                }
                Op::Conditional {
                    check,
                    extends,
                    when_true,
                    when_false,
                    distributive,
                } => json!([
                    "conditional",
                    distributive,
                    child(check),
                    child(extends),
                    child(when_true),
                    child(when_false)
                ]),
                other => panic!("unexpected operator in non-structural fixture: {other:?}"),
            },
            TypeExpr::Tuple(items) => Value::Array(
                std::iter::once(json!("tuple"))
                    .chain(items.iter().map(child))
                    .collect(),
            ),
            TypeExpr::Apply(base, args) if args.len() == 1 => {
                assert!(
                    matches!(base.as_ref(), TypeExpr::Global { name, .. } if Some(*name) == graph.name_id("Array"))
                );
                json!(["array", child(&args[0])])
            }
            TypeExpr::Intrinsic(kind) => json!(["intrinsic", kind.display()]),
            TypeExpr::Literal(LitValue::Number(bits)) => json!(["number", f64::from_bits(*bits)]),
            other => panic!("unexpected source operand: {other:?}"),
        }
    }
    let cases: Vec<Value> = serde_json::from_str(include_str!(
        "../resolution_oracle/operator_type_fixtures.json"
    ))
    .unwrap();
    for grammar in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::LANGUAGE_TSX,
    ] {
        for case in &cases {
            let source = format!(
                "export interface Ops<T, K extends keyof T> {{ value: {}; }}",
                case["syntax"].as_str().unwrap()
            );
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&grammar.into()).unwrap();
            let tree = parser.parse(&source, None).unwrap();
            assert!(!tree.root_node().has_error());
            let graph = crate::indexer::lexical::capture(
                tree.root_node(),
                source.as_bytes(),
                "ts",
                &mut vec![],
                &[],
                crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
            )
            .unwrap();
            assert_eq!(
                shape(
                    graph.types.signatures[1].result.as_ref().unwrap(),
                    graph.types.signatures[0].id,
                    &graph
                ),
                case["shape"],
                "{}",
                case["syntax"]
            );
        }
    }
}
