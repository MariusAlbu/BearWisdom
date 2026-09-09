use super::*;

#[test]
fn legacy_rows_keep_unknown_offsets_and_migration_is_idempotent() {
    let db = crate::Database::open_in_memory().unwrap();
    let conn = db.conn();
    conn.execute_batch(
        "ALTER TABLE ref_resolutions DROP COLUMN source_byte;
         ALTER TABLE ref_resolutions DROP COLUMN source_selector_byte;
         INSERT INTO files(id,path,hash,language,last_indexed) VALUES(1,'a.py','hash','python',0);
         INSERT INTO symbols(id,file_id,name,qualified_name,kind,line,col) VALUES(1,1,'f','f','function',0,0);
         INSERT INTO ref_resolutions(source_id,target_name,kind,source_line,source_col,outcome)
         VALUES(1,'g','calls',1,0,'unresolved');"
    ).unwrap();
    create(conn).unwrap();
    create(conn).unwrap();
    let offset: Option<u32> = conn
        .query_row("SELECT source_byte FROM ref_resolutions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(offset, None);
    let selector: Option<u32> = conn
        .query_row(
            "SELECT source_selector_byte FROM ref_resolutions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(selector, None);
    conn.execute("UPDATE ref_resolutions SET source_byte=0", [])
        .unwrap();
    let offset: Option<u32> = conn
        .query_row("SELECT source_byte FROM ref_resolutions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(offset, Some(0));
}
