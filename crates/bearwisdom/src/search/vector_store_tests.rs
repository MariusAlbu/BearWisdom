use super::*;

fn make_db() -> crate::db::Database {
    crate::db::Database::open_in_memory().unwrap()
}

// -----------------------------------------------------------------------
// vec_to_blob / blob_to_vec — no sqlite-vec required
// -----------------------------------------------------------------------

#[test]
fn vec_to_blob_roundtrip() {
    let original = vec![1.0f32, -2.5, 0.0, 3.14, f32::MIN, f32::MAX];
    let blob = vec_to_blob(&original);
    let recovered = blob_to_vec(&blob);
    assert_eq!(original, recovered);
}

#[test]
fn vec_to_blob_empty() {
    let blob = vec_to_blob(&[]);
    assert!(blob.is_empty());
    assert!(blob_to_vec(&blob).is_empty());
}

#[test]
fn blob_to_vec_wrong_length_returns_empty() {
    let bad = vec![0u8; 7]; // not a multiple of 4
    assert!(blob_to_vec(&bad).is_empty());
}

#[test]
fn vec_to_blob_one_float_correct_bytes() {
    let blob = vec_to_blob(&[1.0f32]);
    // 1.0f32 IEEE 754 LE = 0x3F800000
    assert_eq!(blob, vec![0x00, 0x00, 0x80, 0x3F]);
}

// -----------------------------------------------------------------------
// vec_table_exists / graceful degradation — no sqlite-vec required
// -----------------------------------------------------------------------

#[test]
fn vec_table_not_present_without_extension() {
    let db = make_db();
    assert!(!vec_table_exists(db.conn()));
}

#[test]
fn knn_returns_empty_without_extension() {
    let db = make_db();
    let query = vec![0.0f32; 768];
    let results = knn_search(db.conn(), &query, 10).unwrap();
    assert!(results.is_empty());
}

#[test]
fn delete_file_vectors_noop_without_extension() {
    let db = make_db();
    let deleted = delete_file_vectors(db.conn(), 99).unwrap();
    assert_eq!(deleted, 0);
}

#[test]
fn vector_count_zero_without_extension() {
    let db = make_db();
    let count = vector_count(db.conn()).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn upsert_errors_without_extension() {
    let db = make_db();
    let v = vec![0.0f32; 768];
    let result = upsert_vectors(db.conn(), &[(1, v.as_slice())]);
    assert!(result.is_err(), "upsert should fail without sqlite-vec");
}

// -----------------------------------------------------------------------
// Full integration tests — require SQLITE_VEC_PATH env var
// -----------------------------------------------------------------------

fn try_load_vec(conn: &Connection) -> bool {
    let path = match std::env::var("SQLITE_VEC_PATH") {
        Ok(p) => p,
        Err(_) => return false,
    };
    if unsafe { conn.load_extension_enable() }.is_err() {
        return false;
    }
    let ok = unsafe { conn.load_extension(&path, None) }.is_ok();
    let _ = conn.load_extension_disable(); // not unsafe in rusqlite 0.33
    ok
}

#[test]
#[ignore]
fn init_vec_table_creates_table() {
    let db = make_db();
    if !try_load_vec(db.conn()) {
        eprintln!("Skipping: SQLITE_VEC_PATH not set");
        return;
    }

    let created = init_vec_table(db.conn()).unwrap();
    assert!(created, "Should have created the table");
    assert!(vec_table_exists(db.conn()));

    // Idempotent second call.
    assert!(!init_vec_table(db.conn()).unwrap());
}

#[test]
#[ignore]
fn upsert_and_knn_search_roundtrip() {
    let db = make_db();
    if !try_load_vec(db.conn()) {
        return;
    }
    init_vec_table(db.conn()).unwrap();

    let mut v1 = vec![0.0f32; 768];
    v1[0] = 1.0;
    let mut v2 = vec![0.0f32; 768];
    v2[1] = 1.0;

    upsert_vectors(db.conn(), &[(1, &v1), (2, &v2)]).unwrap();
    assert_eq!(vector_count(db.conn()).unwrap(), 2);

    let results = knn_search(db.conn(), &v1, 2).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, 1, "Nearest chunk should be chunk_id 1");
}

#[test]
#[ignore]
fn delete_file_vectors_removes_rows_via_code_chunks() {
    let db = make_db();
    if !try_load_vec(db.conn()) {
        return;
    }
    init_vec_table(db.conn()).unwrap();

    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('f.rs', 'h', 'rust', 0)",
            [],
        )
        .unwrap();
    let file_id: i64 = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO code_chunks (file_id, content_hash, content, start_line, end_line)
             VALUES (?1, 'x', 'fn f(){}', 0, 0)",
            rusqlite::params![file_id],
        )
        .unwrap();
    let chunk_id: i64 = db.conn().last_insert_rowid();

    let v = vec![0.0f32; 768];
    upsert_vectors(db.conn(), &[(chunk_id, &v)]).unwrap();
    assert_eq!(vector_count(db.conn()).unwrap(), 1);

    let deleted = delete_file_vectors(db.conn(), file_id).unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(vector_count(db.conn()).unwrap(), 0);
}
