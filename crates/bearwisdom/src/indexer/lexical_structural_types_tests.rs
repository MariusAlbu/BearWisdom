use super::*;

#[test]
fn structural_source_types_are_not_legacy_spellings() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../resolution_oracle/structural_type_fixtures.json"
    ))
    .unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let source = format!(
            "export type Shape<T, K extends keyof T> = {};",
            fixture["syntax"].as_str().unwrap()
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.ts");
        std::fs::write(&path, &source).unwrap();
        let file = crate::indexer::parse_file::parse_file_with_arena(
            &crate::walker::WalkedFile {
                relative_path: "main.ts".into(),
                absolute_path: path,
                language: "typescript",
            },
            crate::languages::default_registry(),
            &crate::type_checker::core::types::TypeArena::new(),
        )
        .unwrap();
        let graph = file.flow.lexical.unwrap();
        let recipe = graph.types.aliases.values().next().expect("alias recipe");
        assert!(
            !matches!(recipe, TypeExpr::Legacy(_) | TypeExpr::Unknown),
            "{}: {recipe:?}",
            fixture["name"]
        );
        let mut mapped: Vec<_> = graph
            .types
            .signatures
            .iter()
            .filter(|s| s.declaration.is_none() && s.generics.len() == 1)
            .map(|s| s.id)
            .collect();
        mapped.sort_by_key(|id| id.0.start);
        assert_eq!(
            shape(recipe, &mapped),
            fixture["shape"],
            "{}",
            fixture["name"]
        );
    }
}

fn shape(expr: &TypeExpr, mapped: &[super::signatures::SignatureId]) -> serde_json::Value {
    use crate::type_checker::core::types::{LitValue, MappedModifier, TypeOperator};
    use serde_json::json;
    let child = |expr| shape(expr, mapped);
    let modifier = |m| match m {
        MappedModifier::Preserve => "preserve",
        MappedModifier::Add => "add",
        MappedModifier::Remove => "remove",
    };
    match expr {
        TypeExpr::Parameter {
            owner: Some(_),
            index,
        } => json!(["param", "outer", index]),
        TypeExpr::SignatureParameter { owner, index } => json!([
            "param",
            format!(
                "mapped{}",
                mapped.iter().position(|id| id == owner).unwrap()
            ),
            index
        ]),
        TypeExpr::Literal(LitValue::Str(value)) => json!(["string", value]),
        TypeExpr::Intersection(parts) => {
            let mut values = vec![json!("intersection")];
            values.extend(parts.iter().map(child));
            json!(values)
        }
        TypeExpr::Operator(op) => match op.as_ref() {
            TypeOperator::Object(properties) => {
                let mut values = vec![json!("object")];
                values.extend(properties.iter().map(|p| {
                    let TypeExpr::Literal(LitValue::Str(key)) = &p.key else {
                        panic!("literal key")
                    };
                    assert!(!p.index);
                    json!([key, p.optional, p.readonly, child(&p.value)])
                }));
                json!(values)
            }
            TypeOperator::Mapped {
                parameter,
                keys,
                remap,
                value,
                optional,
                readonly,
            } => {
                let TypeExpr::SignatureParameter { owner, index: 0 } = parameter else {
                    panic!("mapped binder identity")
                };
                assert!(mapped.contains(owner));
                json!([
                    "mapped",
                    modifier(*readonly),
                    modifier(*optional),
                    child(keys),
                    remap.as_ref().map(child),
                    child(value)
                ])
            }
            TypeOperator::KeyOf(ty) => json!(["keyof", child(ty)]),
            TypeOperator::IndexedAccess { object, index } => {
                json!(["index", child(object), child(index)])
            }
            _ => panic!("unexpected operator {op:?}"),
        },
        _ => panic!("unexpected recipe {expr:?}"),
    }
}
