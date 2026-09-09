use super::*;

#[test]
fn elision_sites_are_input_only_and_repeated_capture_uses_the_same_source_anchor() {
    let source = "struct Doc; type Bad = &'_ Doc; fn f(x: &Doc, y: &'_ Doc) -> &'_ Doc { x }
        fn g(gx: &Doc) { let z: &'_ Doc = gx; } fn h(hx: fn(&Doc) -> &Doc) {}";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    let slot = |name| {
        extracted
            .symbols
            .iter()
            .position(|s| s.name == name)
            .unwrap()
    };
    let mut sites = std::collections::HashSet::new();
    for (function, field, index) in [("f", "x", 0), ("f", "y", 1), ("g", "gx", 0)] {
        let mut repeated = Vec::new();
        for recipe in [
            &graph.types.parameters[&slot(function)][index],
            &graph.types.fields[&slot(field)],
        ] {
            let TypeExpr::Indirect {
                region: Some(region),
                ..
            } = recipe
            else {
                panic!("input region");
            };
            let TypeExpr::InputRegion { owner, byte } = region.as_ref() else {
                panic!("source anchor");
            };
            assert_eq!(*owner, slot(function));
            repeated.push((*owner, *byte));
        }
        assert_eq!(repeated[0], repeated[1]);
        assert!(sites.insert(repeated[0]));
    }
    for recipe in [
        &graph.types.aliases[&slot("Bad")],
        &graph.types.fields[&slot("z")],
    ] {
        assert!(
            matches!(recipe, TypeExpr::Indirect { region: None, .. }),
            "non-input elision cannot borrow an arbitrary input ID"
        );
    }
    let TypeExpr::Output { inputs, result } = &graph.types.returns[&slot("f")] else {
        panic!("output relationship");
    };
    assert_eq!(inputs.len(), 2);
    let TypeExpr::Indirect {
        region: Some(region),
        ..
    } = result.as_ref()
    else {
        panic!("output region marker");
    };
    assert!(matches!(region.as_ref(), TypeExpr::OutputRegion));
    assert!(
        matches!(
            graph.types.parameters[&slot("h")][0],
            TypeExpr::Function(_, _)
        ),
        "nested function binder is not the outer input binder"
    );
}

#[test]
fn output_elision_separates_implicit_and_explicit_receiver_binders() {
    for receiver in [
        "&self",
        "&mut self",
        "self",
        "self: Self",
        "self: &Self",
        "self: &mut Self",
    ] {
        let source = format!(
            "struct Doc; struct C; impl C {{ fn f({receiver}, p: &Doc) -> &Doc {{ loop {{}} }} }}"
        );
        let mut extracted = crate::languages::rust_lang::extract::extract(&source);
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let mut data = super::super::capture(
            tree.root_node(),
            source.as_bytes(),
            "rust",
            &extracted.symbols,
            &extracted.refs,
        )
        .unwrap();
        let graph = super::super::capture_locals(
            &mut data,
            tree.root_node(),
            source.as_bytes(),
            "rust",
            &mut extracted.symbols,
            &extracted.refs,
            crate::indexer::flow::BindingSymbols::Synthesize,
        );
        let method = extracted
            .symbols
            .iter()
            .position(|s| s.name == "f")
            .unwrap();
        let TypeExpr::Output { inputs, result } = &graph.types.returns[&method] else {
            panic!("output recipe");
        };
        assert_eq!(
            graph.types.parameters[&method].len(),
            1,
            "explicit self must not shift ordinary arguments"
        );
        if matches!(receiver, "self" | "self: Self") {
            let [TypeExpr::Indirect {
                region: Some(region),
                ..
            }] = inputs.as_slice()
            else {
                panic!("owned self supplies no borrowed region");
            };
            assert!(
                matches!(region.as_ref(), TypeExpr::InputRegion { owner, byte } if *owner == method && *byte as usize >= source.find("p: &Doc").unwrap())
            );
            assert!(!matches!(
                graph.types.receivers[&method],
                TypeExpr::Indirect { .. } | TypeExpr::Unknown
            ));
        } else {
            let [TypeExpr::InputRegion { owner, byte }] = inputs.as_slice() else {
                panic!("{receiver}: receiver region, not ordinary p");
            };
            assert_eq!(*owner, method);
            assert!((*byte as usize) < source.find("p: &Doc").unwrap());
            assert!(matches!(
                graph.types.receivers[&method],
                TypeExpr::Indirect { .. }
            ));
        }
        let TypeExpr::Indirect {
            region: Some(region),
            ..
        } = result.as_ref()
        else {
            panic!("output region");
        };
        assert!(matches!(region.as_ref(), TypeExpr::OutputRegion));
    }
}

#[test]
fn generic_parameter_kinds_preserve_all_positions_and_region_uses_keep_their_owners() {
    use crate::type_checker::core::types::GenericParamKind::{Const, Lifetime, Type};
    let source = "struct Holder<'a,T,const N:usize,U> { left: &'a T, right: U }
        type First<'a> = &'a Holder<'a,u8,1,u16>; type Second<'a> = &'a u8;";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    let slot = |name| {
        extracted
            .symbols
            .iter()
            .position(|s| s.name == name)
            .unwrap()
    };
    let owner = slot("Holder");
    assert_eq!(
        graph.types.generic_declarations[&owner]
            .iter()
            .map(|&(_, kind)| kind)
            .collect::<Vec<_>>(),
        [Lifetime, Type, Const, Type]
    );
    let TypeExpr::Indirect {
        region: Some(region),
        inner,
        ..
    } = &graph.types.fields[&slot("left")]
    else {
        panic!("reference");
    };
    assert!(
        matches!(region.as_ref(), TypeExpr::Parameter { owner: Some(id), index: 0 } if *id == owner)
    );
    assert!(
        matches!(inner.as_ref(), TypeExpr::Parameter { owner: Some(id), index: 1 } if *id == owner)
    );
    assert!(
        matches!(&graph.types.fields[&slot("right")], TypeExpr::Parameter { owner: Some(id), index: 3 } if *id == owner)
    );
    for name in ["First", "Second"] {
        let owner = slot(name);
        let TypeExpr::Indirect {
            region: Some(region),
            ..
        } = &graph.types.aliases[&owner]
        else {
            panic!("named region");
        };
        assert!(
            matches!(region.as_ref(), TypeExpr::Parameter { owner: Some(id), index: 0 } if *id == owner)
        );
    }
}

#[test]
fn exact_arguments_preserve_indirection_and_unattested_lifetimes_never_become_static() {
    use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};
    let source = "struct Item<T> { inner: T } struct Doc;
        type A = Item<&'static mut Doc>; type B<'a> = Item<&'a Doc>; type C = Item<*const Doc>;";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    for (name, kind, mutability) in [
        (
            "A",
            Indirection::Reference(Lifetime::Static),
            Mutability::Mutable,
        ),
        (
            "B",
            Indirection::Reference(Lifetime::Unknown),
            Mutability::Shared,
        ),
        ("C", Indirection::Pointer, Mutability::Shared),
    ] {
        let slot = extracted
            .symbols
            .iter()
            .position(|s| s.name == name)
            .unwrap();
        let TypeExpr::Apply(_, args) = &graph.types.aliases[&slot] else {
            panic!("missing application");
        };
        let TypeExpr::Indirect {
            kind: actual_kind,
            mutability: actual_mutability,
            inner,
            ..
        } = &args[0]
        else {
            panic!("lost indirection: {args:?}");
        };
        assert_eq!((*actual_kind, *actual_mutability), (kind, mutability));
        assert!(matches!(
            inner.as_ref(),
            TypeExpr::Source { legacy: None, .. }
        ));
    }
}

#[test]
fn root_annotation_field_return_and_parameter_recipes_retain_reference_structure() {
    use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};
    let source = "struct Doc; struct Holder { inner: &'static Doc }
        fn make(p: &'static Doc) -> &'static Doc { p }";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    let function = extracted
        .symbols
        .iter()
        .position(|s| s.name == "make")
        .unwrap();
    let field = extracted
        .symbols
        .iter()
        .position(|s| s.name == "inner")
        .unwrap();
    for recipe in [
        &graph.types.fields[&field],
        &graph.types.returns[&function],
        &graph.types.parameters[&function][0],
        graph.types.annotations.values().next().unwrap(),
    ] {
        let recipe = match recipe {
            TypeExpr::Output { result, .. } => result.as_ref(),
            other => other,
        };
        assert!(matches!(
            recipe,
            TypeExpr::Indirect {
                kind: Indirection::Reference(Lifetime::Static),
                mutability: Mutability::Shared,
                ..
            }
        ));
    }
}

#[test]
fn function_types_keep_naked_type_arguments_and_method_parameters_exclude_the_receiver() {
    let source =
        "struct Item; struct C; impl C { fn f(&self, p: Item) -> fn(Item) -> Item { loop {} } }";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    let method = extracted
        .symbols
        .iter()
        .position(|s| s.name == "f")
        .unwrap();
    assert_eq!(graph.types.parameters[&method].len(), 1);
    let TypeExpr::InputApplication {
        owner, base, args, ..
    } = &graph.types.parameters[&method][0]
    else {
        panic!("input type recipe");
    };
    assert_eq!(*owner, method);
    assert!(args.is_empty());
    assert!(matches!(
        base.as_ref(),
        TypeExpr::Source { legacy: None, .. }
    ));
    let TypeExpr::Output { result, .. } = &graph.types.returns[&method] else {
        panic!("output relationship");
    };
    let TypeExpr::Function(args, ret) = result.as_ref() else {
        panic!("function return recipe");
    };
    assert_eq!(args.len(), 1);
    assert!(matches!(args[0], TypeExpr::Source { legacy: None, .. }));
    assert!(matches!(
        ret.as_ref(),
        TypeExpr::Source { legacy: None, .. }
    ));
}

#[test]
fn type_parameters_and_nominal_arguments_use_separate_identity_arenas() {
    let source =
        "struct Holder<T> { inner: T } mod b { pub struct Doc; } fn f(p: Holder<b::Doc>) {}";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    let owner = extracted
        .symbols
        .iter()
        .position(|s| s.name == "Holder")
        .unwrap();
    let field = extracted
        .symbols
        .iter()
        .position(|s| s.name == "inner")
        .unwrap();
    assert!(
        matches!(graph.types.fields[&field], TypeExpr::Parameter { owner: Some(slot), index: 0 } if slot == owner)
    );
    let TypeExpr::InputApplication {
        base: head, args, ..
    } = graph.types.annotations.values().next().unwrap()
    else {
        panic!("missing composite annotation");
    };
    assert!(matches!(
        head.as_ref(),
        TypeExpr::Source { legacy: None, .. }
    ));
    let TypeExpr::InputApplication { base, args, .. } = &args[0] else {
        panic!("nominal argument source recipe");
    };
    assert!(args.is_empty());
    assert!(matches!(
        base.as_ref(),
        TypeExpr::Source { legacy: None, .. }
    ));
    assert_eq!(graph.types.annotations.len(), 1);
}
