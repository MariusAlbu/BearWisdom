#[test]
fn type_heritage_capture_includes_module_and_nested_interfaces_without_globalizing_them() {
    let directory = tempfile::tempdir().unwrap();
    let source = "export {}; interface Root<T> { value: T; } interface Child<T> extends Root<T> {} function nested() { interface Local extends Root<string> {} }";
    let path = directory.path().join("main.ts");
    std::fs::write(&path, source).unwrap();
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
    let graph = file.flow.lexical.as_ref().unwrap();
    assert_eq!(graph.types.interface_bases.len(), 3);
    let globals = graph.globals.as_ref().unwrap();
    assert!(globals.roots.is_empty());
    assert_eq!(globals.interfaces.len(), 3);
    let child = file.symbols.iter().position(|s| s.name == "Child").unwrap();
    let [super::TypeExpr::Apply(base, args)] =
        graph.types.interface_bases[&child].as_deref().unwrap()
    else {
        panic!("generic base missing");
    };
    assert!(matches!(**base, super::TypeExpr::Declaration(_)));
    assert!(
        matches!(args.as_slice(), [super::TypeExpr::Parameter { owner: Some(owner), index: 0 }] if *owner == child)
    );
    assert!(
        graph.types.bases.is_empty(),
        "interface heads must not enter the class value domain"
    );
}
