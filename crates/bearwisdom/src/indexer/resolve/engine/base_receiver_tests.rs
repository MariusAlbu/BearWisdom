use super::*;

#[test]
fn captured_grandparent_arguments_and_returns_share_generic_parameter_ids() {
    use super::super::{compilation::Compilation, substitution::substitute_supertype_args};
    use std::sync::Arc;
    let arena = Arc::new(TypeArena::new());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.ts");
    std::fs::write(&path, "class Doc { touch() {} } class Ancestor<T> { next(): T { throw 0; } } class Parent<U> extends Ancestor<U> {} class Child extends Parent<Doc> {}").unwrap();
    let parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            absolute_path: path,
            relative_path: "a.ts".into(),
            language: "typescript",
        },
        &crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&parsed),
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(&[parsed], &ids, Arc::clone(&arena));
    let id = |name| tree.by_name(name).first().unwrap().id;
    let parent = id("Parent");
    let ancestor = id("Ancestor");
    let child = id("Child");
    let doc = id("Doc");
    let parent_info = tree.canonical_type_info(parent).unwrap();
    let ancestor_info = tree.canonical_type_info(ancestor).unwrap();
    let u = parent_info.generic_param_ids[0];
    let t = ancestor_info.generic_param_ids[0];
    assert_eq!(tree.parent_class_ids(parent), vec![ancestor]);
    assert_eq!(
        tree.parent_class_arg_ids_of(parent, ancestor),
        &[arena.generic_type(u)],
        "edge={:?} expected={:?}; base={:?}",
        tree.parent_class_arg_ids_of(parent, ancestor)
            .iter()
            .map(|&id| arena.get(id))
            .collect::<Vec<_>>(),
        arena.get(arena.generic_type(u)),
        arena.get(parent_info.base_type_id.unwrap())
    );
    let receiver = tree
        .canonical_type_info(child)
        .unwrap()
        .base_type_id
        .unwrap();
    assert_eq!(head_decl_id(&arena, receiver), Some(parent));
    assert_eq!(
        super::super::chain::apply_args(&arena, receiver),
        vec![arena.decl("Doc", doc)]
    );
    let member = tree.by_name("next").first().unwrap().clone();
    let yielded = tree.return_type_id_of(member.id).unwrap();
    assert_eq!(yielded, arena.generic_type(t));
    let result = substitute_supertype_args(&tree, &arena, &member, yielded, receiver, Some(parent));
    assert_eq!(
        result,
        arena.decl("Doc", doc),
        "captured edge IDs are present; member substitution must compose T -> U -> Doc"
    );
}

#[test]
fn persisted_base_recipes_retain_numeric_head_and_type_arguments() {
    let input = Input {
        owner: 7,
        head: Head::Import(19),
        args: vec![],
    };
    let output: Input = serde_json::from_str(&serde_json::to_string(&input).unwrap()).unwrap();
    assert_eq!(output.owner, 7);
    assert!(matches!(output.head, Head::Import(19)));
}
