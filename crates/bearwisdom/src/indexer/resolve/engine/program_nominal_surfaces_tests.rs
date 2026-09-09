use super::super::merge_proof::tests::{context, owner, parse};
use super::*;
use std::{collections::HashSet, sync::Arc};

#[test]
fn nominal_origin_keeps_source_signature_without_a_navigation_row() {
    use crate::indexer::{
        contract_filter::reduce_to_contract, external_parse_payload::CachedParse,
    };
    let original = TypeArena::new();
    let source = "function discard() {} interface Model { value: string; }";
    let mut file = parse(&original, &[("provider.d.ts", source)]).remove(0);
    let discarded = file
        .symbols
        .iter()
        .position(|s| s.name == "discard")
        .unwrap();
    for property in file.symbols.iter_mut().filter(|s| s.name == "value") {
        property.parent_index = Some(discarded);
    }
    reduce_to_contract(&mut file);
    assert!(!file.symbols.iter().any(|s| s.name == "value"));
    let payload = serde_json::to_string(&CachedParse::from_parsed(&file, &original)).unwrap();
    let arena = Arc::new(TypeArena::new());
    arena.class("shift IDs");
    let cached: CachedParse = serde_json::from_str(&payload).unwrap();
    let mut restored =
        cached.into_parsed(&arena, "provider.d.ts", &file.content_hash, file.size, None);
    restored.content = Some(source.into());
    reduce_to_contract(&mut restored);
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&restored),
        "external",
        Some(&arena),
    )
    .unwrap();
    let model = owner(&ids, &restored, "Model");
    let tree = Compilation::build_with_context(
        std::slice::from_ref(&restored),
        &ids,
        Arc::clone(&arena),
        Some(&context(&[&restored])),
        &HashSet::new(),
    );
    let check = |tree: &Compilation| {
        let lookup = tree.program_lookup("provider.d.ts").unwrap();
        let members = &lookup.nominal_surface(model).unwrap().members;
        assert_eq!(members.len(), 1);
        let member = &members[0];
        assert_eq!(member.origin.declaration, None);
        assert_eq!(
            member.origin.signature.0.start,
            source.find("value:").unwrap() as u32
        );
        assert_eq!(
            member.origin.name,
            lookup.member_index().unwrap().name("value")
        );
        assert_eq!(
            tree.type_arena().unwrap().get(member.property.key),
            Type::Literal(LitValue::Str("value".into()))
        );
        assert!(lookup.signature(member.origin.signature).is_some());
    };
    check(&tree);
    tree.persist_type_info(db.conn()).unwrap();
    let cold_arena = Arc::new(TypeArena::new());
    cold_arena.restore_snapshot(&arena.serialize_snapshot());
    let mut cold = Compilation::build(&[], &Default::default(), cold_arena);
    cold.ingest_from_db(db.conn());
    check(&cold);
}
