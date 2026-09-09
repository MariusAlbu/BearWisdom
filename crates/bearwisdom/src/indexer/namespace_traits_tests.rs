use super::*;

fn source(source: &str) -> (NamespaceData, Vec<ExtractedSymbol>) {
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
    super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    (data, extracted.symbols)
}

fn available(data: &NamespaceData, byte: u32) -> Availability {
    let mut scope = Some(data.traits.method_scopes[&byte]);
    let mut result = Availability {
        parent: None,
        bindings: vec![],
        complete: true,
    };
    while let Some(id) = scope {
        let frame = &data.traits.available[&id];
        result.bindings.extend(&frame.bindings);
        result.complete &= frame.complete;
        scope = frame.parent;
    }
    result
}

#[test]
fn trait_declarations_and_empty_or_overridden_implementations_have_separate_source_ids() {
    let (data, symbols) = source(
        "trait Save { fn save(&self) {} } struct A; struct B;
        impl Save for A {} impl Save for B { fn save(&self) {} }",
    );
    assert_eq!(
        data.traits.headers.len(),
        3,
        "trait impls need their own source contract"
    );
    let headers = &data.traits.headers;
    let Owner::Declaration(declaration) = headers[0].owner else {
        panic!("trait declaration");
    };
    assert_eq!(symbols[declaration].kind, crate::types::SymbolKind::Trait);
    assert_eq!(headers[0].members.len(), 1);
    assert_eq!(
        headers[1].members.len(),
        0,
        "empty impl still supplies default-method applicability evidence"
    );
    assert_eq!(headers[2].members.len(), 1);
    assert_ne!(
        headers[0].members[0], headers[2].members[0],
        "static target is not the implementation body"
    );
    assert_ne!(headers[1].owner, headers[2].owner);
    assert!(headers.iter().all(|h| h.enabled));
}

#[test]
fn trait_availability_keeps_shadowed_traits_and_is_shared_by_repeated_selectors() {
    let text = "mod a { pub trait Save {} } mod b { pub trait Save {} }
        use a::Save; fn f(p:Unknown) { p.save(); p.save(); { use b::Save; p.save(); } }
        mod isolated { fn g(p:Unknown) { p.save(); } }";
    let (data, _) = source(text);
    let bytes: Vec<_> = text
        .match_indices("p.save")
        .map(|(byte, _)| byte as u32 + 2)
        .collect();
    assert_eq!(data.traits.method_scopes.len(), 4);
    let environment = |byte| available(&data, byte);
    let type_binding = |environment: &Availability, name: &str| {
        environment
            .bindings
            .iter()
            .copied()
            .filter(|binding| {
                data.entries.iter().any(|((_, id, domain), value)| {
                    *domain == ExportDomain::Type && value == binding && data.spelling(*id) == name
                })
            })
            .collect::<Vec<_>>()
    };
    let outer = type_binding(&environment(bytes[0]), "Save");
    let inner = type_binding(&environment(bytes[2]), "Save");
    assert_eq!(outer.len(), 1);
    assert_eq!(inner.len(), 2);
    assert!(inner.contains(&outer[0]));
    assert_eq!(
        data.traits.method_scopes[&bytes[0]],
        data.traits.method_scopes[&bytes[1]]
    );
    assert!(
        type_binding(&environment(bytes[3]), "Save").is_empty(),
        "module boundaries do not implicitly import parent traits"
    );
}

#[test]
fn anonymous_trait_imports_keep_distinct_bindings_without_defining_an_underscore_name() {
    let text = "mod a { pub trait Save {} } mod b { pub trait Save {} }
        use a::Save as _; use b::Save as _; fn f(p:Unknown) { p.save(); }";
    let (data, _) = source(text);
    let imports: Vec<_> = data
        .traits
        .anonymous_imports
        .values()
        .flatten()
        .copied()
        .collect();
    assert_eq!(imports.len(), 2);
    assert_ne!(imports[0], imports[1]);
    assert!(!data
        .entries
        .iter()
        .any(|((_, name, _), _)| data.spelling(*name) == "_"));
    let available = available(&data, text.find("p.save").unwrap() as u32 + 2);
    assert!(imports
        .iter()
        .all(|binding| available.bindings.contains(binding)));
}

#[test]
fn unknown_glob_provider_and_negative_impl_cannot_be_positive_availability_proofs() {
    let text =
        "trait Save {} struct Doc; impl !Save for Doc {} fn f(p:Doc) { use unknown::*; p.save(); }";
    let (data, _) = source(text);
    assert!(data
        .traits
        .headers
        .iter()
        .any(|h| matches!(h.owner, Owner::Implementation(_)) && h.negative));
    assert!(!available(&data, text.find("p.save").unwrap() as u32 + 2).complete);
}

#[test]
fn trait_method_generic_parameters_have_distinct_signature_scopes() {
    let (data, symbols) = source(
        "trait Get { fn a<T:First>(&self); fn b<T:Second>(&self); } trait First {} trait Second {}",
    );
    let bounds: Vec<_> = data
        .traits
        .bounds
        .iter()
        .filter(|b| matches!(b.owner, Owner::Declaration(_)))
        .collect();
    assert_eq!(bounds.len(), 2);
    let mut owners = Vec::new();
    for bound in bounds {
        let TypeExpr::Parameter {
            owner: Some(owner),
            index: 0,
        } = bound.subject
        else {
            panic!("method-owned parameter: {bound:?}");
        };
        assert_eq!(bound.owner, Owner::Declaration(owner));
        assert_eq!(symbols[owner].kind, crate::types::SymbolKind::Method);
        owners.push(owner);
    }
    assert_ne!(owners[0], owners[1]);
}

#[test]
fn unknown_higher_ranked_bound_is_retained_as_an_obligation() {
    let (data, _) = source(
        "trait Get<T> {} trait Extra {} struct Pair<T>(T);
        impl<T> Get<T> for Pair<T> where T: for<'a> Get<&'a T> + Extra {}",
    );
    let bounds: Vec<_> = data
        .traits
        .bounds
        .iter()
        .filter(|b| matches!(b.owner, Owner::Implementation(_)))
        .collect();
    assert_eq!(bounds.len(), 1);
    assert_eq!(bounds[0].traits.len(), 2);
    assert!(matches!(bounds[0].traits[0], TypeExpr::Unknown));
    assert!(matches!(
        bounds[0].traits[1],
        TypeExpr::Source { legacy: None, .. }
    ));
}

#[test]
fn trait_scope_frames_store_file_bindings_once_across_many_callers() {
    let declarations: String = (0..100).map(|i| format!("trait Trait{i} {{}} ")).collect();
    let callers: String = (0..100)
        .map(|i| format!("fn f{i}(p:Unknown) {{ p.save(); p.save(); }} "))
        .collect();
    let (data, _) = source(&format!("{declarations}{callers}"));
    assert_eq!(data.traits.method_scopes.len(), 200);
    assert_eq!(
        data.traits
            .available
            .values()
            .map(|scope| scope.bindings.len())
            .sum::<usize>(),
        100,
        "store the file's declarations once, not 100 times for its callers"
    );
    for byte in data.traits.method_scopes.keys() {
        assert_eq!(available(&data, *byte).bindings.len(), 100);
    }
}
