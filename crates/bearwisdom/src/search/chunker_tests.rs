use super::*;
use rusqlite::Connection;

fn make_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::schema::apply_pragmas(&conn, true).unwrap();
    crate::db::schema::create_schema(&conn).unwrap();
    conn
}

fn insert_file(conn: &Connection, path: &str) -> i64 {
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'rust', 0)",
        params![path],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn insert_symbol(conn: &Connection, file_id: i64, name: &str, start: u32, end: u32) -> i64 {
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
         VALUES (?1, ?2, ?2, 'function', ?3, 0, ?4)",
        params![file_id, name, start, end],
    )
    .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn sha256_hex_is_deterministic() {
    let h1 = sha256_hex("hello");
    let h2 = sha256_hex("hello");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64); // 32 bytes → 64 hex chars
}

#[test]
fn sha256_hex_differs_for_different_input() {
    assert_ne!(sha256_hex("a"), sha256_hex("b"));
}

#[test]
fn estimate_tokens_rounds_up() {
    assert_eq!(estimate_tokens("abcd"), 1); // 4 chars = 1 token
    assert_eq!(estimate_tokens("abcde"), 2); // 5 chars → 2 tokens
    assert_eq!(estimate_tokens(""), 0);
}

#[test]
fn extract_lines_basic() {
    let lines = vec!["zero", "one", "two", "three"];
    assert_eq!(extract_lines(&lines, 1, 2), "one\ntwo");
}

#[test]
fn extract_lines_out_of_bounds_clamps() {
    let lines = vec!["a", "b"];
    // end beyond length — should clamp
    let result = extract_lines(&lines, 0, 10);
    assert_eq!(result, "a\nb");
}

#[test]
fn chunk_file_no_symbols_produces_single_chunk() {
    let conn = make_db();
    let file_id = insert_file(&conn, "src/empty.rs");
    let content = "fn main() {\n    println!(\"hello\");\n}\n";

    let chunks = chunk_file(&conn, file_id, content, 512).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].symbol_id, None);
    assert_eq!(chunks[0].start_line, 0);
}

#[test]
fn chunk_file_with_symbols_aligns_to_boundaries() {
    let conn = make_db();
    let file_id = insert_file(&conn, "src/two_fns.rs");

    // Two non-overlapping symbols on lines 0-2 and 4-6
    insert_symbol(&conn, file_id, "foo", 0, 2);
    insert_symbol(&conn, file_id, "bar", 4, 6);

    let content = "fn foo() {\n    1\n}\n\nfn bar() {\n    2\n}\n";
    let chunks = chunk_file(&conn, file_id, content, 512).unwrap();

    // We expect at least 2 chunks (one per symbol).
    assert!(chunks.len() >= 2, "Expected at least 2 chunks, got {}", chunks.len());

    // All chunks belong to this file.
    assert!(chunks.iter().all(|c| c.file_id == file_id));
}

#[test]
fn chunk_file_symbol_gets_symbol_id() {
    let conn = make_db();
    let file_id = insert_file(&conn, "src/fn.rs");
    let sym_id = insert_symbol(&conn, file_id, "my_fn", 0, 2);

    let content = "fn my_fn() {\n    42\n}\n";
    let chunks = chunk_file(&conn, file_id, content, 512).unwrap();

    // At least one chunk should carry the symbol id.
    let with_sym: Vec<_> = chunks.iter().filter(|c| c.symbol_id == Some(sym_id)).collect();
    assert!(!with_sym.is_empty(), "At least one chunk should reference the symbol");
}

#[test]
fn chunk_and_store_deletes_and_reinserts() {
    let conn = make_db();
    let file_id = insert_file(&conn, "src/store.rs");

    let content = "fn a() {}\nfn b() {}\n";

    let n1 = chunk_and_store(&conn, file_id, content).unwrap();
    assert!(n1 > 0);

    // Store again — should replace.
    let n2 = chunk_and_store(&conn, file_id, content).unwrap();
    assert_eq!(n1, n2, "Second store should produce same count");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM code_chunks WHERE file_id = ?1", params![file_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, n2 as i64, "DB should contain exactly the new chunks");
}

#[test]
fn chunk_hash_is_stored() {
    let conn = make_db();
    let file_id = insert_file(&conn, "src/hash.rs");
    chunk_and_store(&conn, file_id, "fn x() {}\n").unwrap();

    let hash: String = conn
        .query_row(
            "SELECT content_hash FROM code_chunks WHERE file_id = ?1 LIMIT 1",
            params![file_id],
            |r| r.get(0),
        )
        .unwrap();
    // SHA-256 hex is exactly 64 chars.
    assert_eq!(hash.len(), 64);
}

#[test]
fn oversized_chunk_is_split() {
    let conn = make_db();
    let file_id = insert_file(&conn, "src/big.rs");

    // Create content larger than 512 tokens (> 2048 chars), no symbols.
    let big_line = "x".repeat(200);
    let content = (0..20).map(|_| big_line.clone()).collect::<Vec<_>>().join("\n");
    // 20 lines × 200 chars = 4000 chars → ~1000 tokens > 512 budget

    let chunks = chunk_file(&conn, file_id, &content, 512).unwrap();
    assert!(chunks.len() > 1, "Oversized content should produce multiple chunks");
}

#[test]
fn empty_content_produces_no_chunks() {
    let conn = make_db();
    let file_id = insert_file(&conn, "src/empty.rs");
    let chunks = chunk_file(&conn, file_id, "", 512).unwrap();
    assert!(chunks.is_empty() || chunks.iter().all(|c| c.content.trim().is_empty()),
        "Empty content should produce no meaningful chunks");
}

#[test]
fn chunker_dedupes_identical_symbol_boundaries() {
    // Reproducer for the koreader hang: a single-line file with thousands
    // of symbols all sharing the same (start_line, end_line) caused
    // chunk_file to do O(symbols × file_size) work, blowing up the WAL
    // past 76 GB before being killed.
    //
    // Fix: dedupe by (start_line, end_line) before iterating. One chunk
    // per unique line range still indexes every byte for FTS + embedding;
    // navigation back to a symbol uses the first symbol_id for that range.
    let conn = make_db();
    let file_id = insert_file(&conn, "src/giant_table.lua");

    // Synthesize 5,000 symbols all anchored to line 0 — the shape that
    // wrecked the chunker on koreader's `zh_pinyin_data.lua`.
    for i in 0..5_000 {
        insert_symbol(&conn, file_id, &format!("k{}", i), 0, 0);
    }

    // The "file content" is one fat 200KB line.
    let content = "x".repeat(200_000);

    let t0 = std::time::Instant::now();
    let chunks = chunk_file(&conn, file_id, &content, 512).unwrap();
    let dur = t0.elapsed();

    // Pre-fix: this loop ran 5,000 times, each extracting and re-chunking
    // 200KB. Tens of seconds of CPU. Post-fix: collapses to 1 unique
    // range, sub-second.
    assert!(
        dur < std::time::Duration::from_secs(3),
        "chunker took {:?} on 5k symbols sharing line 0 — boundary dedup likely regressed",
        dur
    );
    // Chunk count is independent of symbol count; bounded by content size /
    // max_tokens. 200KB ÷ ~2KB-per-chunk-of-512-tokens ≈ ~100 chunks.
    assert!(
        chunks.len() < 200,
        "expected one chunk-set per unique range, not per symbol; got {}",
        chunks.len()
    );
}

#[test]
fn chunker_preserves_distinct_symbol_boundaries() {
    // Sanity: the dedup key is (start_line, end_line). Symbols with
    // distinct ranges must each get their own chunks attached.
    let conn = make_db();
    let file_id = insert_file(&conn, "src/three_funcs.rs");
    insert_symbol(&conn, file_id, "f1", 0, 2);
    insert_symbol(&conn, file_id, "f2", 4, 6);
    insert_symbol(&conn, file_id, "f3", 8, 10);
    let content = (0..15).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let chunks = chunk_file(&conn, file_id, &content, 512).unwrap();
    let symbol_ids: std::collections::HashSet<_> = chunks
        .iter()
        .filter_map(|c| c.symbol_id)
        .collect();
    assert_eq!(symbol_ids.len(), 3, "each distinct range must get its own symbol_id-attached chunk");
}
