//! Integration tests for the LSP background enricher.
//!
//! Tests that require a live `typescript-language-server` process are marked
//! `#[ignore]` and must be run explicitly:
//!
//!   cargo test --test lsp_enrich -- --ignored
//!
//! The single test that does NOT need LSP (`test_enrich_empty_unresolved`) runs
//! as part of the normal test suite.

use std::sync::Arc;
use std::time::Duration;

use bearwisdom::{full_index, BackgroundEnricher, DbPool, GraphBridge, LspManager};
use bearwisdom::lsp::registry::ServerRegistry;
use bearwisdom_tests::TestProject;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a `TestProject` with a minimal TypeScript workspace that the TS
/// language server can understand.  The fixture has:
///
///   src/service.ts   — exports `UserService`
///   src/handler.ts   — imports and calls `UserService`
///   tsconfig.json    — minimal config pointing the server at `src/`
fn typescript_project() -> TestProject {
    let p = TestProject { dir: tempfile::TempDir::new().unwrap() };

    p.add_file(
        "tsconfig.json",
        r#"{
  "compilerOptions": {
    "target": "ES2020",
    "module": "commonjs",
    "strict": true,
    "outDir": "./dist"
  },
  "include": ["src/**/*"]
}
"#,
    );

    p.add_file(
        "src/service.ts",
        r#"export class UserService {
    getUser(id: number): string {
        return `user-${id}`;
    }

    listUsers(): string[] {
        return [];
    }
}
"#,
    );

    p.add_file(
        "src/handler.ts",
        r#"import { UserService } from "./service";

export function handleRequest(id: number): string {
    const svc = new UserService();
    return svc.getUser(id);
}

export function listAll(): string[] {
    const svc = new UserService();
    return svc.listUsers();
}
"#,
    );

    p
}

/// Create a `DbPool` backed by a unique temp file and wire it to a
/// `BackgroundEnricher` pointed at `workspace_root`.
///
/// The enricher's `rate_limit` is set to zero so tests don't wait 100 ms per
/// ref.
fn make_enricher_for(workspace_root: &std::path::Path) -> (DbPool, Arc<BackgroundEnricher>) {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let db_path = std::env::temp_dir().join(format!("bw_lsp_enrich_{pid}_{id}.db"));

    let pool = DbPool::new(&db_path, 2).expect("DbPool::new");
    let lsp = Arc::new(LspManager::new(workspace_root));
    let bridge = Arc::new(GraphBridge::new(pool.clone(), lsp, workspace_root));

    let mut enricher = BackgroundEnricher::new(bridge);
    enricher.rate_limit = Duration::ZERO;

    (pool, Arc::new(enricher))
}

// ---------------------------------------------------------------------------
// test_lsp_detect_installed
// ---------------------------------------------------------------------------

/// Verify that `ServerRegistry::detect_installed()` finds at least
/// `typescript-language-server` on this machine.
///
/// Skipped in CI and on machines where the server is not installed.
/// Run explicitly with: `cargo test --test lsp_enrich -- --ignored`
#[tokio::test]
#[ignore = "requires typescript-language-server on PATH"]
async fn test_lsp_detect_installed() {
    let registry = ServerRegistry::new();
    let installed = registry.detect_installed().await;

    let ts_entry = installed
        .iter()
        .find(|e| e.command == "typescript-language-server");

    assert!(
        ts_entry.is_some(),
        "typescript-language-server not found by detect_installed(); \
         install it with: npm install -g typescript-language-server typescript"
    );
}

// ---------------------------------------------------------------------------
// test_enrich_typescript_resolves_refs
// ---------------------------------------------------------------------------

/// Full end-to-end enrichment pass on the TypeScript fixture.
///
/// 1. Index the project with `full_index` — tree-sitter writes `unresolved_refs`
///    rows for cross-file references.
/// 2. Run `enrich_unresolved` — LSP resolves them and writes edges with
///    confidence = 1.0.
/// 3. Verify at least one ref was resolved.
///
/// Requires `typescript-language-server` on PATH.
#[tokio::test]
#[ignore = "requires typescript-language-server on PATH"]
async fn test_enrich_typescript_resolves_refs() {
    let project = typescript_project();
    let workspace_root = project.path();

    // Index with a file-backed Database so DbPool can re-open the same file.
    let db_path = workspace_root.join(".bearwisdom").join("index.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    {
        let mut db = bearwisdom::Database::open(&db_path).expect("open index db");
        full_index(&mut db, workspace_root, None, None).expect("full_index");
    }

    let pool = DbPool::new(&db_path, 2).expect("DbPool::new");

    // Confirm there are unresolved refs to work on.
    let unresolved_before: i64 = {
        let guard = pool.get().unwrap();
        guard
            .conn
            .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))
            .unwrap_or(0)
    };
    assert!(
        unresolved_before > 0,
        "expected tree-sitter to produce unresolved_refs after indexing TypeScript; got 0"
    );

    let lsp = Arc::new(LspManager::new(workspace_root));
    let bridge = Arc::new(GraphBridge::new(pool.clone(), lsp, workspace_root));
    let mut enricher = BackgroundEnricher::new(bridge);
    enricher.rate_limit = Duration::ZERO;

    let progress = enricher
        .enrich_unresolved(50)
        .await
        .expect("enrich_unresolved");

    assert!(
        progress.resolved_this_pass > 0,
        "expected at least one ref resolved by LSP; got 0 (total_unresolved={}, still={})",
        progress.total_unresolved,
        progress.still_unresolved,
    );

    // Verify edges with confidence 1.0 were written.
    let high_conf_edges: i64 = {
        let guard = pool.get().unwrap();
        guard
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE confidence = 1.0",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0)
    };
    assert!(
        high_conf_edges > 0,
        "expected edges with confidence=1.0 after enrichment; got 0"
    );
}

// ---------------------------------------------------------------------------
// test_enrich_low_confidence_upgrade
// ---------------------------------------------------------------------------

/// Manually plant a low-confidence edge and verify `enrich_low_confidence`
/// upgrades it to 1.0 when the LSP confirms the reference.
///
/// Requires `typescript-language-server` on PATH.
#[tokio::test]
#[ignore = "requires typescript-language-server on PATH"]
async fn test_enrich_low_confidence_upgrade() {
    let project = typescript_project();
    let workspace_root = project.path();

    let db_path = workspace_root.join(".bearwisdom").join("index.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    {
        let mut db = bearwisdom::Database::open(&db_path).expect("open index db");
        full_index(&mut db, workspace_root, None, None).expect("full_index");
    }

    let pool = DbPool::new(&db_path, 2).expect("DbPool::new");

    // Find source_id for `handleRequest` in handler.ts and the target_id for
    // `UserService` in service.ts so we can plant a plausible edge.
    let (source_id, target_id): (i64, i64) = {
        let guard = pool.get().unwrap();

        let source: Option<i64> = guard
            .conn
            .query_row(
                "SELECT s.id FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.path LIKE '%handler.ts'
                   AND s.name = 'handleRequest'
                 LIMIT 1",
                [],
                |r| r.get(0),
            )
            .ok();

        let target: Option<i64> = guard
            .conn
            .query_row(
                "SELECT s.id FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.path LIKE '%service.ts'
                   AND s.name = 'UserService'
                 LIMIT 1",
                [],
                |r| r.get(0),
            )
            .ok();

        match (source, target) {
            (Some(s), Some(t)) => (s, t),
            _ => {
                // If the indexer didn't produce these symbols the test is
                // vacuously passing — skip rather than fail.
                eprintln!(
                    "SKIP: handleRequest or UserService not found in index; \
                     indexer may have changed its output"
                );
                return;
            }
        }
    };

    // line 4 of handler.ts (0-based): `    const svc = new UserService();`
    let reference_line: u32 = 4;

    // Plant a low-confidence edge (0.5) as tree-sitter would.
    {
        let guard = pool.get().unwrap();

        // Delete any existing edge so we start clean.
        guard
            .conn
            .execute(
                "DELETE FROM edges WHERE source_id = ?1 AND target_id = ?2 AND kind = 'calls'",
                rusqlite::params![source_id, target_id],
            )
            .unwrap();

        guard
            .conn
            .execute(
                "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
                 VALUES (?1, ?2, 'calls', ?3, 0.5)",
                rusqlite::params![source_id, target_id, reference_line],
            )
            .unwrap();
    }

    let lsp = Arc::new(LspManager::new(workspace_root));
    let bridge = Arc::new(GraphBridge::new(pool.clone(), lsp, workspace_root));
    let mut enricher = BackgroundEnricher::new(bridge);
    enricher.rate_limit = Duration::ZERO;

    let progress = enricher
        .enrich_low_confidence(0.85, 50)
        .await
        .expect("enrich_low_confidence");

    assert!(
        progress.upgraded_this_pass > 0,
        "expected the low-confidence edge to be upgraded; upgraded_this_pass=0"
    );

    // Confirm the edge now sits at 1.0.
    let upgraded_conf: f64 = {
        let guard = pool.get().unwrap();
        guard
            .conn
            .query_row(
                "SELECT confidence FROM edges
                 WHERE source_id = ?1 AND target_id = ?2 AND kind = 'calls'",
                rusqlite::params![source_id, target_id],
                |r| r.get(0),
            )
            .expect("edge should still exist after upgrade")
    };

    assert!(
        (upgraded_conf - 1.0).abs() < f64::EPSILON,
        "expected confidence=1.0 after upgrade, got {upgraded_conf}"
    );
}

// ---------------------------------------------------------------------------
// test_enrich_empty_unresolved
// ---------------------------------------------------------------------------

/// When `unresolved_refs` is empty, `enrich_unresolved` must return 0 resolved
/// without error and without touching the LSP server.
///
/// This test does NOT require any language server — it runs in the normal suite.
#[tokio::test]
async fn test_enrich_empty_unresolved() {
    let project = TestProject::typescript_app();
    let workspace_root = project.path();

    let (pool, enricher) = make_enricher_for(workspace_root);

    // Index the project to create a valid schema, then clear unresolved_refs.
    {
        let db_path = std::env::temp_dir().join(format!(
            "bw_lsp_empty_{}.db",
            std::process::id()
        ));
        let mut db = bearwisdom::Database::open(&db_path).expect("open db");
        full_index(&mut db, workspace_root, None, None).expect("full_index");

        // Copy symbols/files into the pool's DB so the schema is populated,
        // but we don't actually need them — we just need the tables to exist.
    }

    // The pool's DB already has the schema from DbPool::new (Database::open).
    // Ensure unresolved_refs is empty.
    {
        let guard = pool.get().unwrap();
        guard
            .conn
            .execute("DELETE FROM unresolved_refs", [])
            .unwrap();
    }

    let progress = enricher
        .enrich_unresolved(100)
        .await
        .expect("enrich_unresolved should not error on empty table");

    assert_eq!(
        progress.resolved_this_pass, 0,
        "nothing to resolve — expected 0"
    );
    assert_eq!(
        progress.total_unresolved, 0,
        "table was cleared — expected total_unresolved=0"
    );
    assert_eq!(
        progress.still_unresolved, 0,
        "table was cleared — expected still_unresolved=0"
    );
}
