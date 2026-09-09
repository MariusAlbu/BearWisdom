use std::collections::HashMap;
use std::sync::Arc;

use super::*;
use crate::indexer::resolve::engine::{
    compilation::Compilation,
    file_context::build_profiles,
    pipeline::resolve_one_file,
    semantic_model::SemanticModel,
    testkit_fixtures::{call_ref, source_symbol},
};
use crate::occurrence::OccurrenceCounts;
use crate::type_checker::core::types::TypeArena;
use crate::types::FlowMeta;

fn file(language: &str) -> ParsedFile {
    ParsedFile {
        path: "fixture.source".into(),
        language: language.into(),
        content_hash: "hash".into(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: vec![
            source_symbol("caller"),
            source_symbol("helper"),
            source_symbol("orphan"),
        ],
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

fn measured(file: &ParsedFile) -> (OccurrenceCounts, Vec<OccurrenceBucket>) {
    let ids: crate::indexer::write::SymbolIds = HashMap::from([
        ((file.path.clone(), "caller".into()), 1),
        ((file.path.clone(), "helper".into()), 2),
    ])
    .into();
    let tree = Compilation::build(std::slice::from_ref(file), &ids, Arc::new(TypeArena::new()));
    let (_, _, log, census) = resolve_one_file(
        file,
        &tree,
        &build_profiles(),
        &FxHashMap::default(),
        None,
        &SemanticModel::production(),
        &ids,
        None,
    );
    let buckets = census.buckets();
    let mut counts = OccurrenceCounts::default();
    for bucket in &buckets {
        counts.add(bucket.disposition, bucket.count);
    }
    assert_eq!(
        counts.total(),
        file.refs.len() as u64,
        "every extraction must have one disposition"
    );
    assert_eq!(
        counts.resolved + counts.unresolved + counts.drained,
        log.len() as u64
    );
    (counts, buckets)
}

#[test]
fn every_early_exit_is_counted_and_same_line_columns_survive() {
    let mut pf = file("shell");
    pf.refs = vec![
        call_ref("helper"),
        call_ref("helper"),
        call_ref("helper"),
        call_ref("missing_xyz"),
        call_ref("echo"),
        call_ref("_primitive"),
        call_ref("invalid_owner"),
        call_ref("missing_id"),
    ];
    pf.refs[2].col = 12;
    pf.refs[2].byte_offset = 12;
    pf.refs[6].source_symbol_index = 99;
    pf.refs[7].source_symbol_index = 2;
    let (counts, _) = measured(&pf);
    assert_eq!(counts.resolved, 2);
    assert_eq!(counts.duplicate, 1);
    assert_eq!(counts.unresolved, 1);
    assert_eq!(counts.drained, 1);
    assert_eq!(counts.primitive, 1);
    assert_eq!(counts.missing_source_symbol, 1);
    assert_eq!(counts.missing_source_id, 1);
}

#[test]
fn symbol_less_files_and_unsupported_profiles_are_visible() {
    let mut pf = file("ruby");
    pf.symbols.clear();
    pf.refs.push(call_ref("puts"));
    let (counts, _) = measured(&pf);
    assert_eq!(counts.missing_source_symbol, 1);
    assert_eq!(counts.binding_coverage_percent(), Some(0.0));
    pf.language = "not_a_registered_language".into();
    let (counts, _) = measured(&pf);
    assert_eq!(counts.unsupported_language, 1);
}

#[test]
fn sample_and_embedded_origin_metadata_apply_to_resolved_refs_too() {
    let mut pf = file("typescript");
    pf.refs.push(call_ref("helper"));
    pf.symbol_from_snippet = vec![true];
    pf.ref_origin_languages = vec![Some("javascript".into())];
    let (counts, buckets) = measured(&pf);
    assert_eq!(counts.resolved, 1);
    assert_eq!(buckets[0].language, "javascript");
    assert!(buckets[0].from_snippet);
}

#[test]
fn primitive_annotation_is_accounted_even_when_it_is_not_logged() {
    let mut pf = file("typescript");
    let mut r = call_ref("string");
    r.kind = EdgeKind::TypeRef;
    pf.refs.push(r);
    let (counts, _) = measured(&pf);
    assert_eq!(counts.primitive, 1);
}

#[test]
fn persistence_rejects_an_incomplete_partition() {
    let db = crate::db::Database::open_in_memory().unwrap();
    let mut pf = file("typescript");
    pf.refs.push(call_ref("helper"));
    let census = FileCensus::new(&pf);
    let tx = db.conn().unchecked_transaction().unwrap();
    let error = persist(&tx, &[census], false).unwrap_err();
    assert!(error.to_string().contains("Incomplete occurrence census"));
}

#[test]
fn failed_census_rolls_back_the_resolution_transaction() {
    let mut db = crate::db::Database::open_in_memory().unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO files (id, path, hash, language, last_indexed)
         VALUES (1, 'fixture.source', 'hash', 'typescript', 0);
         INSERT INTO symbols (id, file_id, name, qualified_name, kind, line, col)
         VALUES (1, 1, 'caller', 'caller', 'function', 0, 0);
         INSERT INTO edges (source_id, target_id, kind, source_line, confidence, strategy)
         VALUES (1, 1, 'calls', 0, 1.0, 'before');",
        )
        .unwrap();
    let mut pf = file("typescript");
    pf.refs.push(call_ref("helper"));
    let census = FileCensus::new(&pf); // Intentionally incomplete.
    assert!(crate::indexer::resolve::engine::flush::flush_to_db(
        &mut db,
        &[],
        &[],
        &[],
        &[census],
        true,
    )
    .is_err());
    let strategy: String = db
        .conn()
        .query_row("SELECT strategy FROM edges", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        strategy, "before",
        "clearing the old graph must roll back on failure"
    );
}

#[test]
fn missing_file_cannot_allocate_an_id_and_claim_an_unrelated_file() {
    let db = crate::db::Database::open_in_memory().unwrap();
    db.conn()
        .execute(
            "INSERT INTO files (id, path, hash, language, last_indexed)
         VALUES (1, 'unrelated.ts', 'hash', 'typescript', 0)",
            [],
        )
        .unwrap();
    let pf = file("typescript");
    let census = FileCensus::new(&pf);
    let tx = db.conn().unchecked_transaction().unwrap();
    let error = persist(&tx, &[census], false).unwrap_err();
    assert!(error.to_string().contains("Census file is not indexed"));
    let rows: u64 = tx
        .query_row("SELECT COUNT(*) FROM resolution_census", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}
