// =============================================================================
// connectors/dotnet_di.rs  —  .NET Dependency Injection connector
//
// Detects AddScoped<I,T>, AddTransient<I,T>, AddSingleton<I,T> registrations
// in C# files and creates `implements` edges between interface and
// implementation symbols.
//
// Two forms are handled:
//   AddScoped<ICatalogService, CatalogService>()  → interface + impl
//   AddScoped<CatalogService>()                   → impl only (self-registration)
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single DI registration detected in a C# file.
#[derive(Debug, Clone)]
pub struct DiRegistration {
    /// `files.id` of the file containing the registration call.
    pub file_id: i64,
    /// 1-based line number of the call.
    pub line: u32,
    /// Lifetime of the registration.
    pub lifetime: String,
    /// The interface type — for the single-type form this equals `implementation_type`.
    pub interface_type: String,
    /// The concrete implementation type.
    pub implementation_type: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan all indexed C# files for DI registrations.
///
/// Returns detected registrations with file_id and line metadata.
/// Files are read from disk via `project_root`.
pub fn detect_di_registrations(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<DiRegistration>> {
    let re_two = build_two_type_regex();
    let re_one = build_one_type_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'csharp'")
        .context("Failed to prepare C# files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query C# files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect C# file rows")?;

    let mut registrations: Vec<DiRegistration> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable C# file");
                continue;
            }
        };

        detect_in_source(&source, file_id, &re_two, &re_one, &mut registrations);
    }

    debug!(count = registrations.len(), "DI registrations detected");
    Ok(registrations)
}

/// Link detected DI registrations to the symbol graph.
///
/// For two-type registrations: creates an `implements` edge from the
/// implementation class to the interface.  For single-type registrations
/// (self-registration) no edge is created — the type implements itself,
/// which is trivially true and not useful in the graph.
///
/// Returns the number of edges inserted.
pub fn link_di_registrations(
    conn: &Connection,
    registrations: &[DiRegistration],
) -> Result<u32> {
    let mut created: u32 = 0;

    for reg in registrations {
        // Skip self-registrations — no interface/impl distinction.
        if reg.interface_type == reg.implementation_type {
            continue;
        }

        let iface_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM symbols WHERE name = ?1 AND kind = 'interface' LIMIT 1",
                [&reg.interface_type],
                |r| r.get(0),
            )
            .optional();

        let impl_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM symbols WHERE name = ?1 AND kind = 'class' LIMIT 1",
                [&reg.implementation_type],
                |r| r.get(0),
            )
            .optional();

        let (iface_id, impl_id) = match (iface_id, impl_id) {
            (Some(i), Some(c)) => (i, c),
            _ => {
                debug!(
                    interface = %reg.interface_type,
                    implementation = %reg.implementation_type,
                    "Skipping DI registration — symbol(s) not found"
                );
                continue;
            }
        };

        let metadata = serde_json::json!({
            "lifetime": reg.lifetime,
            "source": "di_registration",
        })
        .to_string();

        let result = conn.execute(
            "INSERT OR IGNORE INTO edges
                (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'implements', ?3, 0.90)",
            rusqlite::params![impl_id, iface_id, reg.line],
        );

        match result {
            Ok(n) if n > 0 => {
                created += 1;
                // Also add a flow_edge for cross-language tracing dashboards.
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, confidence, metadata
                     ) SELECT
                        si.file_id, ?3, ?4, 'csharp',
                        sf.file_id, NULL, ?5, 'csharp',
                        'di_binding', 0.90, ?6
                     FROM symbols si, symbols sf
                     WHERE si.id = ?1 AND sf.id = ?2",
                    rusqlite::params![impl_id, iface_id, reg.line, reg.implementation_type, reg.interface_type, metadata],
                );
            }
            Ok(_) => {} // OR IGNORE — duplicate
            Err(e) => {
                debug!(err = %e, "Failed to insert implements edge for DI registration");
            }
        }
    }

    info!(
        created,
        "DI connector: linked registrations to symbol graph"
    );
    Ok(created)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Regex for the two-type form: `Add{Lifetime}<Interface, Implementation>`.
fn build_two_type_regex() -> Regex {
    // Captures: (1) lifetime, (2) interface, (3) implementation
    Regex::new(r"Add(Scoped|Transient|Singleton)\s*<\s*(\w+)\s*,\s*(\w+)\s*>")
        .expect("two-type DI regex is valid")
}

/// Regex for the one-type form: `Add{Lifetime}<Implementation>`.
fn build_one_type_regex() -> Regex {
    // Captures: (1) lifetime, (2) type
    Regex::new(r"Add(Scoped|Transient|Singleton)\s*<\s*(\w+)\s*>")
        .expect("one-type DI regex is valid")
}

fn detect_in_source(
    source: &str,
    file_id: i64,
    re_two: &Regex,
    re_one: &Regex,
    out: &mut Vec<DiRegistration>,
) {
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // Two-type form takes priority — try it first.
        if let Some(cap) = re_two.captures(line_text) {
            out.push(DiRegistration {
                file_id,
                line: line_no,
                lifetime: cap[1].to_lowercase(),
                interface_type: cap[2].to_string(),
                implementation_type: cap[3].to_string(),
            });
            continue; // Only one registration per line.
        }

        // One-type form.
        if let Some(cap) = re_one.captures(line_text) {
            let type_name = cap[2].to_string();
            out.push(DiRegistration {
                file_id,
                line: line_no,
                lifetime: cap[1].to_lowercase(),
                interface_type: type_name.clone(),
                implementation_type: type_name,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Extension trait for rusqlite::Connection
// ---------------------------------------------------------------------------

trait OptionalExt<T> {
    fn optional(self) -> Option<T>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    // -----------------------------------------------------------------------
    // Unit tests for source detection
    // -----------------------------------------------------------------------

    #[test]
    fn detects_two_type_scoped() {
        let re_two = build_two_type_regex();
        let re_one = build_one_type_regex();
        let mut out = Vec::new();
        detect_in_source(
            r#"services.AddScoped<ICatalogService, CatalogService>();"#,
            1,
            &re_two,
            &re_one,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].lifetime, "scoped");
        assert_eq!(out[0].interface_type, "ICatalogService");
        assert_eq!(out[0].implementation_type, "CatalogService");
    }

    #[test]
    fn detects_two_type_transient() {
        let re_two = build_two_type_regex();
        let re_one = build_one_type_regex();
        let mut out = Vec::new();
        detect_in_source(
            r#"services.AddTransient<IOrderService, OrderService>();"#,
            2,
            &re_two,
            &re_one,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].lifetime, "transient");
        assert_eq!(out[0].interface_type, "IOrderService");
        assert_eq!(out[0].implementation_type, "OrderService");
    }

    #[test]
    fn detects_two_type_singleton() {
        let re_two = build_two_type_regex();
        let re_one = build_one_type_regex();
        let mut out = Vec::new();
        detect_in_source(
            r#"services.AddSingleton<ICache, RedisCache>();"#,
            3,
            &re_two,
            &re_one,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].lifetime, "singleton");
        assert_eq!(out[0].interface_type, "ICache");
        assert_eq!(out[0].implementation_type, "RedisCache");
    }

    #[test]
    fn detects_one_type_form() {
        let re_two = build_two_type_regex();
        let re_one = build_one_type_regex();
        let mut out = Vec::new();
        detect_in_source(
            r#"services.AddScoped<CatalogService>();"#,
            5,
            &re_two,
            &re_one,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].interface_type, "CatalogService");
        assert_eq!(out[0].implementation_type, "CatalogService");
    }

    #[test]
    fn two_type_takes_priority_over_one_type() {
        // The two-type regex matches first; the line should not produce two entries.
        let re_two = build_two_type_regex();
        let re_one = build_one_type_regex();
        let mut out = Vec::new();
        detect_in_source(
            r#"services.AddScoped<IFoo, Foo>();"#,
            1,
            &re_two,
            &re_one,
            &mut out,
        );
        assert_eq!(out.len(), 1, "Two-type match should not also emit a one-type match");
        assert_eq!(out[0].interface_type, "IFoo");
    }

    #[test]
    fn empty_source_produces_no_registrations() {
        let re_two = build_two_type_regex();
        let re_one = build_one_type_regex();
        let mut out = Vec::new();
        detect_in_source("// no registrations here", 1, &re_two, &re_one, &mut out);
        assert!(out.is_empty());
    }

    // -----------------------------------------------------------------------
    // Integration tests against in-memory DB
    // -----------------------------------------------------------------------

    fn seed_symbols(db: &Database) -> (i64, i64) {
        let conn = &db.conn;

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('Services.cs', 'h1', 'csharp', 0)",
            [],
        )
        .unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'ICatalogService', 'App.ICatalogService', 'interface', 5, 0)",
            [file_id],
        )
        .unwrap();
        let iface_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'CatalogService', 'App.CatalogService', 'class', 20, 0)",
            [file_id],
        )
        .unwrap();
        let impl_id: i64 = conn.last_insert_rowid();

        (iface_id, impl_id)
    }

    #[test]
    fn link_creates_implements_edge() {
        let db = Database::open_in_memory().unwrap();
        let (_, impl_id) = seed_symbols(&db);

        // Get the file_id from the impl symbol.
        let file_id: i64 = db
            .conn
            .query_row(
                "SELECT file_id FROM symbols WHERE id = ?1",
                [impl_id],
                |r| r.get(0),
            )
            .unwrap();

        let registrations = vec![DiRegistration {
            file_id,
            line: 42,
            lifetime: "scoped".to_string(),
            interface_type: "ICatalogService".to_string(),
            implementation_type: "CatalogService".to_string(),
        }];

        let created = link_di_registrations(&db.conn, &registrations).unwrap();
        assert_eq!(created, 1, "Expected one implements edge");

        let count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edges WHERE kind = 'implements'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn self_registration_creates_no_edge() {
        let db = Database::open_in_memory().unwrap();
        seed_symbols(&db);

        let registrations = vec![DiRegistration {
            file_id: 1,
            line: 10,
            lifetime: "scoped".to_string(),
            interface_type: "CatalogService".to_string(),
            implementation_type: "CatalogService".to_string(),
        }];

        let created = link_di_registrations(&db.conn, &registrations).unwrap();
        assert_eq!(created, 0, "Self-registration should produce no edge");
    }

    #[test]
    fn missing_symbols_skipped_without_error() {
        let db = Database::open_in_memory().unwrap();

        let registrations = vec![DiRegistration {
            file_id: 1,
            line: 5,
            lifetime: "transient".to_string(),
            interface_type: "INonExistent".to_string(),
            implementation_type: "AlsoNonExistent".to_string(),
        }];

        // Should not panic or return Err.
        let created = link_di_registrations(&db.conn, &registrations).unwrap();
        assert_eq!(created, 0);
    }

    #[test]
    fn duplicate_registration_not_double_counted() {
        let db = Database::open_in_memory().unwrap();
        let (_, impl_id) = seed_symbols(&db);

        let file_id: i64 = db
            .conn
            .query_row(
                "SELECT file_id FROM symbols WHERE id = ?1",
                [impl_id],
                |r| r.get(0),
            )
            .unwrap();

        let reg = DiRegistration {
            file_id,
            line: 42,
            lifetime: "scoped".to_string(),
            interface_type: "ICatalogService".to_string(),
            implementation_type: "CatalogService".to_string(),
        };

        link_di_registrations(&db.conn, &[reg.clone()]).unwrap();
        let created = link_di_registrations(&db.conn, &[reg]).unwrap();
        assert_eq!(created, 0, "OR IGNORE should prevent duplicate edge");
    }
}
