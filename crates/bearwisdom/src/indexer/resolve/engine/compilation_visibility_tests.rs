use super::*;

#[test]
fn visibility_closure_uses_ids_not_names_and_terminates_on_cycles() {
    let arena = Arc::new(TypeArena::new());
    let mut tree = Compilation::build(&[], &SymbolIds::default(), arena);
    tree.lexical_only.insert(1);
    tree.members_by_id.insert(1, vec![2]);
    tree.members_by_id.insert(2, vec![1, 3]);
    tree.members_by_id.insert(4, vec![5]);
    tree.expand_lexical_visibility();
    assert_eq!(tree.lexical_only, [1, 2, 3].into_iter().collect());
}

#[test]
fn fresh_id_metadata_does_not_suppress_a_same_named_database_declaration() {
    let db = crate::Database::open_in_memory().unwrap();
    let arena = Arc::new(TypeArena::new());
    let old = arena.decl("Same", 71);
    let fresh = arena.decl("Same", 72);
    let other = arena.decl("Same", 73);
    db.conn()
        .execute_batch(
            "INSERT INTO files (id,path,hash,language,last_indexed)
        VALUES (10,'a.ts','a','typescript',0),(20,'b.ts','b','typescript',0);
        INSERT INTO symbols (id,file_id,name,qualified_name,kind,line,col)
        VALUES (1,10,'run','run','function',0,0);",
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO symbol_type_info (symbol_id,return_type_id) VALUES (1,?1)",
            [old.0.get()],
        )
        .unwrap();
    let mut tree = Compilation::build(&[], &SymbolIds::default(), arena);
    tree.ingest_from_db(db.conn());
    tree.type_info_by_id.get_mut(&1).unwrap().return_type_id = Some(fresh);
    db.conn()
        .execute_batch(
            "INSERT INTO symbols (id,file_id,name,qualified_name,kind,line,col)
        VALUES (2,20,'run','run','function',0,0);",
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO symbol_type_info (symbol_id,return_type_id) VALUES (2,?1)",
            [other.0.get()],
        )
        .unwrap();
    tree.ingest_from_db(db.conn());
    assert_eq!(tree.by_name("run").len(), 2);
    assert_eq!(tree.return_type_id_of(1), Some(fresh));
    assert_eq!(tree.return_type_id_of(2), Some(other));
}

#[test]
fn reloaded_private_rows_are_hidden_from_roots_but_keep_id_member_lookup() {
    use crate::indexer::resolve::engine::file_lookup::FileLookup;
    let db = crate::Database::open_in_memory().unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO files (id,path,hash,language,last_indexed)
        VALUES (10,'private.ts','a','typescript',0),(20,'public.ts','b','typescript',0);
        INSERT INTO symbols (id,file_id,name,qualified_name,kind,line,col,containing_id)
        VALUES (1,10,'Hidden','Hidden','class',0,0,NULL),
               (2,10,'save','Hidden.save','method',1,0,1),
               (3,20,'Hidden','Hidden','class',0,0,NULL);",
        )
        .unwrap();
    crate::db::lexical_visibility::replace(db.conn(), 10, [1]).unwrap();
    let mut tree = Compilation::build(&[], &SymbolIds::default(), Arc::new(TypeArena::new()));
    tree.ingest_from_db(db.conn());
    let lookup = FileLookup::new(&tree, "typescript");
    assert_eq!(
        lookup
            .by_name("Hidden")
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(lookup.by_qualified_name("Hidden").map(|s| s.id), Some(3));
    assert_eq!(
        lookup
            .types_by_name("Hidden")
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [3]
    );
    assert!(lookup.in_file("private.ts").is_empty());
    assert!(lookup.by_name("save").is_empty());
    assert_eq!(lookup.symbol_by_id(1).unwrap().id, 1);
    assert_eq!(
        lookup
            .members_of_id(1)
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        [2]
    );
    crate::db::lexical_visibility::replace(db.conn(), 10, []).unwrap();
    let mut restored = Compilation::build(&[], &SymbolIds::default(), Arc::new(TypeArena::new()));
    restored.ingest_from_db(db.conn());
    assert!(
        !restored.lexical_only.contains(&1),
        "a removed restriction must not survive reload"
    );
}
