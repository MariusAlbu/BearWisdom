use super::*;

fn fixture() -> Database {
    let db = Database::open_in_memory().unwrap();
    db.conn().execute_batch(
        "INSERT INTO files(id,path,hash,language,last_indexed) VALUES(9,'a.js','hash','javascript',0);
         INSERT INTO symbols(id,file_id,name,qualified_name,kind,line,col)
         VALUES(30,9,'caller','caller','function',0,0),(40,9,'target','target','function',2,0);
         INSERT INTO ref_resolutions(source_id,target_name,kind,source_line,source_col,source_byte,outcome,target_id,confidence)
         VALUES(30,'target','calls',1,0,15,'resolved',40,1.0),
               (30,'target','calls',1,0,25,'resolved',40,1.0);"
    ).unwrap();
    db
}

fn sites() -> Vec<ReferenceSite> {
    [15, 25, 35]
        .map(|byte_offset| ReferenceSite {
            file: FixtureFileId(1),
            byte_offset,
            kind: EdgeKind::Calls,
        })
        .to_vec()
}

fn read_fixture(db: &Database) -> QueryResult<Vec<Observation>> {
    observations(
        db,
        &HashMap::from([(9, FixtureFileId(1))]),
        &[EdgeKind::Calls],
        &sites(),
    )
}

#[test]
fn byte_offsets_distinguish_same_line_zero_column_calls_and_preserve_skips() {
    let evidence = read_fixture(&fixture()).unwrap();
    assert_eq!(evidence.len(), 3);
    assert_ne!(evidence[0].site, evidence[1].site);
    assert_eq!(
        evidence[0].binding,
        Some(ObservedBinding::Resolved(DeclarationSite {
            file: FixtureFileId(1),
            line: 2,
            col: 0,
            kind: crate::types::SymbolKind::Function,
        }))
    );
    assert_eq!(evidence[0].binding, evidence[1].binding);
    assert_eq!(evidence[2].binding, None);
}

#[test]
fn legacy_positions_are_not_silently_assumed_to_be_zero() {
    let db = fixture();
    db.conn()
        .execute("UPDATE ref_resolutions SET source_byte=NULL", [])
        .unwrap();
    assert!(read_fixture(&db)
        .unwrap_err()
        .to_string()
        .contains("Legacy reference"));
}

#[test]
fn co_located_logs_and_duplicate_manifest_ids_are_rejected() {
    let db = fixture();
    db.conn()
        .execute("UPDATE ref_resolutions SET source_byte=15", [])
        .unwrap();
    assert!(read_fixture(&db)
        .unwrap_err()
        .to_string()
        .contains("Ambiguous resolution log"));
    let files = HashMap::from([(9, FixtureFileId(1)), (10, FixtureFileId(1))]);
    assert!(observations(&db, &files, &[EdgeKind::Calls], &sites()).is_err());
}

#[test]
fn deleted_target_is_not_a_successful_binding() {
    let db = fixture();
    db.conn()
        .execute("DELETE FROM symbols WHERE id=40", [])
        .unwrap();
    let evidence = read_fixture(&db).unwrap();
    assert_eq!(evidence[0].binding, Some(ObservedBinding::DanglingTarget));
}

#[test]
fn log_from_different_extraction_snapshot_is_rejected() {
    let db = fixture();
    db.conn()
        .execute(
            "UPDATE ref_resolutions SET source_byte=99 WHERE source_byte=15",
            [],
        )
        .unwrap();
    assert!(read_fixture(&db)
        .unwrap_err()
        .to_string()
        .contains("absent from extraction input"));
}

#[test]
fn display_names_and_confidence_do_not_change_identity() {
    let db = fixture();
    let before = read_fixture(&db).unwrap();
    db.conn()
        .execute_batch(
            "UPDATE symbols SET name='renamed', qualified_name='display_only';
         UPDATE ref_resolutions SET target_name='anything', confidence=0.01, strategy='different';",
        )
        .unwrap();
    assert_eq!(before, read_fixture(&db).unwrap());
}

#[test]
fn co_located_declarations_are_not_silently_assumed_equal() {
    let db = fixture();
    db.conn()
        .execute_batch(
            "INSERT INTO symbols(id,file_id,name,qualified_name,kind,line,col)
         VALUES(50,9,'synthetic','synthetic','function',2,0);",
        )
        .unwrap();
    assert!(read_fixture(&db)
        .unwrap_err()
        .to_string()
        .contains("Ambiguous declaration"));
}

#[test]
fn selector_anchors_distinguish_nested_calls_without_target_names() {
    let db = fixture();
    db.conn().execute_batch(
        "UPDATE ref_resolutions SET source_selector_byte=source_byte, source_byte=10, target_name='same';"
    ).unwrap();
    let files = HashMap::from([(9, FixtureFileId(1))]);
    let rows = selector_observations(&db, &files, &[EdgeKind::Calls], &sites()).unwrap();
    assert_eq!(rows[0].site.byte_offset, 15);
    assert_eq!(rows[1].site.byte_offset, 25);
    assert_eq!(rows[0].binding, rows[1].binding);
    db.conn()
        .execute("UPDATE ref_resolutions SET source_selector_byte=NULL", [])
        .unwrap();
    assert!(selector_observations(&db, &files, &[EdgeKind::Calls], &sites()).is_err());
}
