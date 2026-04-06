use super::*;
use crate::db::DbPool;
use crate::lsp::manager::LspManager;

fn test_db_path() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("bw_enricher_test_{pid}_{id}.db"))
}

fn make_enricher() -> BackgroundEnricher {
    let path = test_db_path();
    let pool = DbPool::new(&path, 2).unwrap();
    let lsp = Arc::new(LspManager::new("/tmp/test-workspace"));
    let bridge = Arc::new(GraphBridge::new(pool, lsp, "/tmp/test-workspace"));
    BackgroundEnricher::new(bridge)
}

#[test]
fn test_enrichment_progress_default() {
    let p = EnrichmentProgress::default();
    assert_eq!(p.total_unresolved, 0);
    assert_eq!(p.resolved_this_pass, 0);
    assert_eq!(p.upgraded_this_pass, 0);
    assert_eq!(p.still_unresolved, 0);
    assert_eq!(p.elapsed_ms, 0);
}

#[test]
fn test_cancel_flag() {
    let enricher = make_enricher();
    assert!(!enricher.is_cancelled());
    enricher.cancel();
    assert!(enricher.is_cancelled());
}

#[test]
fn test_new_defaults() {
    let enricher = make_enricher();
    assert_eq!(enricher.rate_limit, Duration::from_millis(100));
    assert!(!enricher.is_cancelled());
}

/// Verify the SQL issued by `enrich_unresolved` selects `ur.target_name`.
/// We confirm this by seeding a row with a known target_name and checking
/// that a full enrichment pass reads it without panicking — the query
/// would fail at compile time if the column reference were wrong, and at
/// runtime (via `row.get(5)`) if the column index were off.
#[tokio::test]
async fn test_enrich_unresolved_reads_target_name() {
    let path = test_db_path();
    let pool = DbPool::new(&path, 2).unwrap();
    let lsp = Arc::new(LspManager::new("/tmp/test-workspace"));
    let bridge = Arc::new(GraphBridge::new(pool.clone(), lsp, "/tmp/test-workspace"));
    let enricher = BackgroundEnricher::new(bridge);

    // Seed the minimum schema rows needed.
    {
        let db = pool.get().unwrap();
        db.conn().execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES ('src/a.ts', 'h', 'typescript', 0)",
            [],
        ).unwrap();
        let file_id = db.conn().last_insert_rowid();

        db.conn().execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
             VALUES (?1, 'myFunc', 'mod::myFunc', 'function', 3, 0, 10)",
            [file_id],
        ).unwrap();
        let sym_id = db.conn().last_insert_rowid();

        db.conn().execute(
            "INSERT INTO unresolved_refs (source_id, target_name, kind, source_line)
             VALUES (?1, 'otherFunc', 'calls', 5)",
            [sym_id],
        ).unwrap();
    }

    // Run a pass — it will try to read the file from disk (which won't
    // exist in /tmp/test-workspace), so it will skip the LSP call.  What
    // matters is that the query doesn't panic or return an error.
    let result = enricher.enrich_unresolved(10).await;
    assert!(result.is_ok(), "enrich_unresolved returned error: {result:?}");

    let progress = result.unwrap();
    // Nothing was resolved (no real LSP / file), but the query ran cleanly.
    assert_eq!(progress.resolved_this_pass, 0);
    assert_eq!(progress.total_unresolved, 1);
}
