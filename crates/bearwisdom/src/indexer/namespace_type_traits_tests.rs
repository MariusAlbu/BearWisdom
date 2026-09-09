use super::*;

#[test]
fn applicability_recipes_do_not_keep_legacy_name_fallbacks() {
    let recipe = TypeExpr::Apply(
        Box::new(TypeExpr::Source {
            usage: Use {
                binding: BindingId(7),
                domain: ExportDomain::Type,
                local: false,
            },
            legacy: Some("external::Trait".into()),
        }),
        vec![TypeExpr::Legacy("T::Item".into())],
    );
    let TypeExpr::Apply(base, args) = strict(recipe) else {
        panic!("application");
    };
    assert!(matches!(
        *base,
        TypeExpr::Source {
            usage: Use {
                binding: BindingId(7),
                local: true,
                ..
            },
            legacy: None
        }
    ));
    assert!(matches!(args.as_slice(), [TypeExpr::Unknown]));
}
