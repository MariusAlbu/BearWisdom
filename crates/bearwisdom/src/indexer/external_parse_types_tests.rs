use super::*;

#[test]
fn content_only_cache_never_recreates_a_program_nominal_from_its_display_name() {
    let source = TypeArena::new();
    let destination = TypeArena::new();
    let context = crate::type_checker::core::types::NominalContextId::fresh();
    let nominal = source.decl_in(context, "Poisoned", 71);
    let applied = source.intern(Type::Apply {
        base: nominal,
        args: vec![source.primitive(PrimKind::Bool)],
    });
    assert_eq!(
        destination.get(roundtrip(&source, nominal, &destination)),
        Type::Unknown
    );
    let Type::Apply { base, .. } = destination.get(roundtrip(&source, applied, &destination))
    else {
        panic!()
    };
    assert_eq!(destination.get(base), Type::Unknown);
    assert!(destination.class_lookup("Poisoned").is_none());
}

#[test]
fn content_only_cache_does_not_export_runtime_caller_ids_as_portable_regions() {
    let source = TypeArena::new();
    let destination = TypeArena::new();
    let region = Lifetime::Inference {
        owner: 71,
        byte: 44,
    };
    let bare = source.intern(Type::Region(region));
    let reference = source.intern(Type::Indirect {
        kind: Indirection::Reference(region),
        mutability: Mutability::Shared,
        inner: source.primitive(PrimKind::Bool),
    });
    assert_eq!(
        destination.get(roundtrip(&source, bare, &destination)),
        Type::Region(Lifetime::Unknown)
    );
    let Type::Indirect { kind, .. } = destination.get(roundtrip(&source, reference, &destination))
    else {
        panic!("reference shape retained");
    };
    assert_eq!(kind, Indirection::Reference(Lifetime::Unknown));
}

#[test]
fn portable_regions_use_parameter_table_indices_not_source_arena_ids() {
    let src = TypeArena::new();
    src.intern_type_parameter("noise".into(), 0, None);
    let p = src.intern_generic(GenericParamData {
        name: "'a".into(),
        kind: GenericParamKind::Lifetime,
        owner_symbol_index: 8,
        bound: None,
    });
    let reference = src.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Parameter(p)),
        mutability: Mutability::Shared,
        inner: src.class("Doc"),
    });
    let mut exporter = TypeExporter::new(&src);
    let cached = exporter.export(reference);
    let arg = exporter.export(src.generic_type(p));
    let params = exporter.into_params();
    assert_eq!(arg, CachedType::Region(CachedLifetime::Parameter(0)));
    assert_eq!(params.len(), 1);
    let wire = serde_json::to_string(&(cached, arg, params)).unwrap();
    let (cached, arg, params) =
        serde_json::from_str::<(CachedType, CachedType, Vec<CachedGenericParam>)>(&wire).unwrap();
    let dst = TypeArena::new();
    for _ in 0..4 {
        dst.intern_type_parameter("'a".into(), 8, None);
    }
    let mut importer = TypeImporter::new(&dst, params);
    let imported = importer.import(&cached);
    let arg = importer.import(&arg);
    let Type::Indirect {
        kind: Indirection::Reference(Lifetime::Parameter(mapped)),
        ..
    } = dst.get(imported)
    else {
        panic!("reference region");
    };
    assert_ne!(mapped, p);
    assert_eq!(dst.get(arg), Type::Region(Lifetime::Parameter(mapped)));
    assert_eq!(dst.generic_param(mapped).kind, GenericParamKind::Lifetime);
    assert_eq!(dst.format_type(imported), "&'a Doc");
    let bad = importer.import(&CachedType::Region(CachedLifetime::Parameter(90)));
    assert_eq!(dst.get(bad), Type::Region(Lifetime::Unknown));
}

/// Round-trip a TypeId through export → serde → import into a FRESH arena and
/// return the id there. Equal structure interns to equal ids, so callers can
/// compare against an expected id built directly in the destination arena.
fn roundtrip(src: &TypeArena, id: TypeId, dst: &TypeArena) -> TypeId {
    let mut ex = TypeExporter::new(src);
    let cached = ex.export(id);
    let json = serde_json::to_string(&cached).expect("serialize");
    let back: CachedType = serde_json::from_str(&json).expect("deserialize");
    let mut im = TypeImporter::new(dst, ex.into_params());
    im.import(&back)
}

#[test]
fn class_and_apply_roundtrip_structurally() {
    let src = TypeArena::new();
    let id = src.intern_type_str("Vec<tantivy.Document>");
    let dst = TypeArena::new();
    let got = roundtrip(&src, id, &dst);
    let expected = dst.intern_type_str("Vec<tantivy.Document>");
    assert_eq!(got, expected);
    // The qname→class index must be populated by the import, matching what a
    // fresh extraction's `class()` calls produce.
    assert!(dst.class_lookup("Vec").is_some());
    assert!(dst.class_lookup("tantivy.Document").is_some());
}

#[test]
fn every_structural_variant_roundtrips() {
    let src = TypeArena::new();
    let user = src.class("User");
    let cases = vec![
        src.intern(Type::Intrinsic(Intrinsic::Unknown)),
        src.intern(Type::Intrinsic(Intrinsic::String)),
        src.intern(Type::Literal(LitValue::Number(1.5f64.to_bits()))),
        src.intern(Type::Literal(LitValue::BigInt {
            negative: true,
            words: vec![0, 0, 1],
        })),
        src.intern(Type::Literal(LitValue::Utf16(vec![0xd800]))),
        src.primitive(PrimKind::Int),
        src.intern(Type::Tuple(vec![user, src.primitive(PrimKind::Str)])),
        src.intern(Type::Union(vec![user, src.primitive(PrimKind::Unit)])),
        src.intern(Type::Intersection(vec![user, src.class("Base")])),
        src.intern(Type::Function {
            params: vec![user],
            return_: src.primitive(PrimKind::Bool),
        }),
        src.intern(Type::Optional(user)),
        src.intern(Type::AsyncWrapper(user)),
        src.intern(Type::Iterator(user)),
        src.intern(Type::Indirect {
            kind: Indirection::Pointer,
            mutability: Mutability::Mutable,
            inner: user,
        }),
        src.intern(Type::Indirect {
            kind: Indirection::Reference(crate::type_checker::core::types::Lifetime::Static),
            mutability: Mutability::Shared,
            inner: user,
        }),
        src.intern(Type::Literal(LitValue::Str("k".into()))),
        src.intern(Type::Unknown),
    ];
    let dst = TypeArena::new();
    for id in cases {
        let got = roundtrip(&src, id, &dst);
        // Deep structural equality: re-exporting from the destination arena
        // must produce the identical portable tree.
        let mut ex_src = TypeExporter::new(&src);
        let mut ex_dst = TypeExporter::new(&dst);
        assert_eq!(ex_src.export(id), ex_dst.export(got));
    }
}

#[test]
fn generic_param_identity_is_preserved() {
    let src = TypeArena::new();
    let bound = src.class("Animal");
    let t = src.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: 3,
        bound: Some(bound),
    });
    let generic_t = src.intern(Type::Generic { param: t });
    // Two independent occurrences of the same parameter: inside an Apply and
    // as a bare Generic.
    let vec_t = src.intern(Type::Apply {
        base: src.class("Vec"),
        args: vec![generic_t],
    });

    let mut ex = TypeExporter::new(&src);
    let cached_bare = ex.export(generic_t);
    let cached_apply = ex.export(vec_t);
    let params = ex.into_params();
    assert_eq!(params.len(), 1, "one parameter, one table entry");
    assert_eq!(params[0].name, "T");
    assert_eq!(params[0].owner_symbol_index, 3);

    let dst = TypeArena::new();
    let mut im = TypeImporter::new(&dst, params);
    let got_bare = im.import(&cached_bare);
    let got_apply = im.import(&cached_apply);
    let Type::Generic { param: p1 } = dst.get(got_bare) else {
        panic!("expected Generic");
    };
    let Type::Apply { args, .. } = dst.get(got_apply) else {
        panic!("expected Apply");
    };
    let Type::Generic { param: p2 } = dst.get(args[0]) else {
        panic!("expected Generic arg");
    };
    assert_eq!(
        p1, p2,
        "both occurrences must re-intern to ONE GenericParamId"
    );
    let data = dst.generic_param(p1);
    assert_eq!(data.name, "T");
    assert_eq!(data.owner_symbol_index, 3);
    let bound_id = data.bound.expect("bound must survive");
    assert_eq!(dst.get(bound_id), Type::Class("Animal".into()));
}

#[test]
fn bound_referencing_another_parameter_roundtrips() {
    // `U extends Comparable<T>` — a bound whose tree contains a DIFFERENT
    // generic parameter, so importing U's bound recursively interns T.
    let src = TypeArena::new();
    let t = src.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".into(),
        owner_symbol_index: 0,
        bound: None,
    });
    let generic_t = src.intern(Type::Generic { param: t });
    let comparable_t = src.intern(Type::Apply {
        base: src.class("Comparable"),
        args: vec![generic_t],
    });
    let u = src.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "U".into(),
        owner_symbol_index: 0,
        bound: Some(comparable_t),
    });
    let id = src.intern(Type::Generic { param: u });

    let dst = TypeArena::new();
    let got = roundtrip(&src, id, &dst);
    let Type::Generic { param } = dst.get(got) else {
        panic!("expected Generic");
    };
    let data = dst.generic_param(param);
    assert_eq!(data.name, "U");
    let Type::Apply { args, .. } = dst.get(data.bound.expect("bound survives")) else {
        panic!("expected Apply bound");
    };
    let Type::Generic { param: inner_t } = dst.get(args[0]) else {
        panic!("expected Generic inside bound");
    };
    assert_eq!(dst.generic_param(inner_t).name, "T");
}

#[test]
fn out_of_range_param_index_fails_soft() {
    let dst = TypeArena::new();
    let mut im = TypeImporter::new(&dst, Vec::new());
    let id = im.import(&CachedType::Generic(7));
    let Type::Generic { param } = dst.get(id) else {
        panic!("expected Generic");
    };
    assert_eq!(dst.generic_param(param).name, "");
}

#[test]
fn deferred_type_operators_roundtrip_without_losing_shared_parameter_ownership() {
    use crate::type_checker::core::types::TypeOperator;
    let src = TypeArena::new();
    let p = src.intern_generic(GenericParamData {
        name: "T".into(),
        owner_symbol_index: 3,
        kind: GenericParamKind::Type,
        bound: None,
    });
    let generic = src.generic_type(p);
    let key = src.intern(Type::Operator(TypeOperator::KeyOf(generic)));
    let indexed = src.intern(Type::Operator(TypeOperator::IndexedAccess {
        object: generic,
        index: key,
    }));
    let ty = src.intern(Type::Operator(TypeOperator::Readonly(indexed)));
    let dst = TypeArena::new();
    dst.intern(Type::Unknown);
    let got = roundtrip(&src, ty, &dst);
    let Type::Operator(TypeOperator::Readonly(inner)) = dst.get(got) else {
        panic!()
    };
    let Type::Operator(TypeOperator::IndexedAccess { object, index }) = dst.get(inner) else {
        panic!()
    };
    assert_eq!(dst.get(index), Type::Operator(TypeOperator::KeyOf(object)));
    let Type::Generic { param } = dst.get(object) else {
        panic!()
    };
    assert_eq!(dst.generic_param(param).owner_symbol_index, 3);
}
