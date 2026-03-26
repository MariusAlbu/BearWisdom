use super::*;
use std::path::PathBuf;

#[test]
fn embed_chunks_no_vec_extension() {
    let db = crate::db::Database::open_in_memory().unwrap();
    let mut embedder = Embedder::new(PathBuf::from("/nonexistent"));
    // Should return (0, 0) gracefully — no sqlite-vec loaded.
    let result = embed_chunks(&db.conn, &mut embedder, 32).unwrap();
    assert_eq!(result, (0, 0));
}
