use super::*;

#[test]
fn detached_bodies_reserve_self_and_member_slots_before_forward_import_lowering() {
    let source = "impl Renamed { pub fn new() -> Self { Self } } use model::Item as Renamed; mod model { pub struct Item; }";
    let extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let data = capture(
        tree.root_node(),
        source.as_bytes(),
        syntax_for("rust").unwrap(),
        &extracted.symbols,
        &extracted.refs,
    );
    let [extension] = data.extensions.as_slice() else {
        panic!("missing owner recipe");
    };
    assert_eq!(extension.members.len(), 1);
    assert_eq!(extracted.symbols[extension.members[0]].name, "new");
    assert!(matches!(
        data.bindings[extension.owner.0].targets.as_slice(),
        [Target::Binding(_)]
    ));
}

#[test]
fn member_access_records_use_source_slots_and_container_policy() {
    let source = "pub trait View { fn read(&self); fn provided(&self) {} }
        pub struct Item { pub field: u32, private: u32 }
        impl View for Item { fn read(&self) {} }
        impl Item { fn hidden(&self) {} }
        #[cfg(feature=\"extra\")] impl Item { pub fn optional(&self) {} }
        pub enum Choice { A { value: u32 } }";
    let extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let data = capture(
        tree.root_node(),
        source.as_bytes(),
        syntax_for("rust").unwrap(),
        &extracted.symbols,
        &extracted.refs,
    );
    let facts: Vec<_> = data
        .declaration_access
        .iter()
        .map(|a| (&extracted.symbols[a.slot], &a.scope))
        .collect();
    // Enum payload fields are not emitted as declaration slots by the current
    // extractor; this test checks access only for source-correlated items.
    for name in ["field", "read", "provided"] {
        let captured: Vec<_> = facts.iter().filter(|(s, _)| s.name == name).collect();
        assert!(!captured.is_empty(), "missing {name}: {facts:?}");
        assert!(
            captured.iter().all(|(_, scope)| scope.is_none()),
            "implicit/explicit public {name}: {facts:?}"
        );
    }
    for name in ["hidden", "private"] {
        assert!(facts
            .iter()
            .any(|(s, scope)| s.name == name
                && matches!(scope, Some(Target::Module(SourceModuleId(0))))));
    }
    assert!(facts
        .iter()
        .any(|(s, scope)| s.name == "optional" && matches!(scope, Some(Target::Missing))));
}

#[test]
fn qualified_call_sites_and_alias_paths_are_captured_from_source_not_rewritten_names() {
    let source = "mod real { pub struct Model; impl Model { pub fn new() -> Self { Self } } } use real::Model as Alias; fn f() { Alias::new(); }";
    let extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let data = capture(
        tree.root_node(),
        source.as_bytes(),
        syntax_for("rust").unwrap(),
        &extracted.symbols,
        &extracted.refs,
    );
    assert_eq!(data.units.len(), 2);
    assert_eq!(data.roots.len(), 1);
    assert!(data.roots.values().all(|r| r.local));
}

#[test]
fn missing_bare_value_keeps_an_authoritative_unknown_recipe() {
    let source = "mod unrelated { pub fn absent() {} } fn f() { absent(); }";
    let extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let data = capture(
        tree.root_node(),
        source.as_bytes(),
        syntax_for("rust").unwrap(),
        &extracted.symbols,
        &extracted.refs,
    );
    let usage = data.roots.values().next().unwrap();
    assert!(matches!(
        data.bindings[usage.binding.0].targets.as_slice(),
        [Target::External(_)]
    ));
    assert!(
        usage.local,
        "Captured bare-value miss cannot disappear into unrestricted same-file/global lookup"
    );
}
