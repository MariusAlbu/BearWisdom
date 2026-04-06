// =============================================================================
// connectors/django.rs  —  Django framework connector
//
// Three detection passes over indexed Python files:
//
//   1. Models — scan for `class Foo(models.Model)` declarations.  The matching
//      symbol is annotated with a "django-models" concept.
//
//   2. URL patterns — scan files whose path ends in `urls.py` for
//      `path("route", view)` calls.  The view reference is looked up in the
//      symbols table and a row is inserted into `routes`.
//
//   3. Views — scan for class-based views (`class Foo(SomeView)`) and function-
//      based views (`def foo(request`).  Matching symbols are annotated with a
//      "django-views" concept.
//
// All detection is regex-based.  Django conventions are regular enough that
// full AST parsing would add cost without meaningful accuracy gain.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::db::Database;

// ---------------------------------------------------------------------------
// Regex builders
// ---------------------------------------------------------------------------

fn build_model_regex() -> Regex {
    Regex::new(r"class\s+(\w+)\s*\(\s*models\.Model\s*\)")
        .expect("django model regex is valid")
}

fn build_url_path_regex() -> Regex {
    // Matches: path('route', view_fn) or path("route", view_fn)
    // Also re_path variants with the same structure.
    Regex::new(r#"(?:re_)?path\s*\(\s*['"]([^'"]+)['"]\s*,\s*(\w[\w.]*)"#)
        .expect("django url path regex is valid")
}

fn build_cbv_regex() -> Regex {
    // Class-based view: class Foo(SomeView) or class Foo(View)
    Regex::new(r"class\s+(\w+)\s*\([^)]*View[^)]*\)")
        .expect("django cbv regex is valid")
}

fn build_fbv_regex() -> Regex {
    // Function-based view: def foo(request  ...
    Regex::new(r"def\s+(\w+)\s*\(\s*request")
        .expect("django fbv regex is valid")
}

// ---------------------------------------------------------------------------
// Internal helpers — query helpers
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
// Step 1: Django model detection
// ---------------------------------------------------------------------------

fn detect_django_models(conn: &Connection, project_root: &Path) -> Result<u32> {
    let re_model = build_model_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'python'")
        .context("Failed to prepare Python file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Python files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Python file rows")?;

    let mut concept_count: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Python file");
                continue;
            }
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re_model.captures_iter(line_text) {
                let class_name = &cap[1];

                // Find the matching symbol in the DB.
                let symbol_id: Option<i64> = conn
                    .query_row(
                        "SELECT id FROM symbols
                         WHERE file_id = ?1 AND name = ?2 AND kind = 'class'
                         LIMIT 1",
                        rusqlite::params![file_id, class_name],
                        |r| r.get(0),
                    )
                    .optional();

                debug!(
                    class = class_name,
                    line = line_no,
                    symbol_id = ?symbol_id,
                    "Django model detected"
                );

                // Insert a concept membership edge in flow_edges using the
                // same pattern as other connectors (source = symbol, edge_type
                // carries the concept name, target = self for concept anchor).
                let result = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, protocol, confidence
                     ) VALUES (
                        ?1, ?2, ?3, 'python',
                        ?1, ?2, ?3, 'python',
                        'django_model', 'orm', 0.95
                     )",
                    rusqlite::params![file_id, line_no, class_name],
                );

                match result {
                    Ok(n) if n > 0 => concept_count += 1,
                    Ok(_) => {}
                    Err(e) => debug!(err = %e, "Failed to insert django_model flow_edge"),
                }
            }
        }
    }

    Ok(concept_count)
}

// ---------------------------------------------------------------------------
// Step 2: URL pattern detection → routes table
// ---------------------------------------------------------------------------

fn detect_django_urls(conn: &Connection, project_root: &Path) -> Result<u32> {
    let re_url = build_url_path_regex();

    // Only look at urls.py files.
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language = 'python' AND (path LIKE '%urls.py' OR path = 'urls.py')",
        )
        .context("Failed to prepare urls.py query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query urls.py files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect urls.py rows")?;

    let mut route_count: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable urls.py");
                continue;
            }
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re_url.captures_iter(line_text) {
                let route_path = &cap[1];
                // The view reference may be dotted (e.g. `views.my_view` or
                // `MyView.as_view()`).  Take the last component as the name.
                let view_ref = &cap[2];
                let view_name = view_ref.split('.').next_back().unwrap_or(view_ref);

                // Try to find the view symbol by name anywhere in Python files.
                let symbol_id: Option<i64> = conn
                    .query_row(
                        "SELECT s.id FROM symbols s
                         JOIN files f ON f.id = s.file_id
                         WHERE s.name = ?1 AND f.language = 'python'
                           AND s.kind IN ('function', 'class', 'method')
                         LIMIT 1",
                        rusqlite::params![view_name],
                        |r| r.get(0),
                    )
                    .optional();

                debug!(
                    route = route_path,
                    view = view_name,
                    "Django URL pattern detected"
                );

                // Insert into routes. Django paths handle all methods; we
                // record GET as the canonical method per the spec.
                let result = conn.execute(
                    "INSERT OR IGNORE INTO routes
                       (file_id, symbol_id, http_method, route_template, resolved_route, line)
                     VALUES (?1, ?2, 'GET', ?3, ?3, ?4)",
                    rusqlite::params![file_id, symbol_id, route_path, line_no],
                );

                match result {
                    Ok(n) if n > 0 => route_count += 1,
                    Ok(_) => {}
                    Err(e) => debug!(err = %e, "Failed to insert Django route"),
                }
            }
        }
    }

    Ok(route_count)
}

// ---------------------------------------------------------------------------
// Step 3: View detection
// ---------------------------------------------------------------------------

fn detect_django_views(conn: &Connection, project_root: &Path) -> Result<u32> {
    let re_cbv = build_cbv_regex();
    let re_fbv = build_fbv_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'python'")
        .context("Failed to prepare Python file query for views")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Python files for views")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Python file rows")?;

    let mut view_count: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;

            // Class-based views.
            for cap in re_cbv.captures_iter(line_text) {
                let class_name = &cap[1];
                let result = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, protocol, confidence
                     ) VALUES (
                        ?1, ?2, ?3, 'python',
                        ?1, ?2, ?3, 'python',
                        'django_view', 'http', 0.90
                     )",
                    rusqlite::params![file_id, line_no, class_name],
                );
                match result {
                    Ok(n) if n > 0 => view_count += 1,
                    Ok(_) => {}
                    Err(e) => debug!(err = %e, class = class_name, "Failed to insert django_view (cbv)"),
                }
            }

            // Function-based views.
            for cap in re_fbv.captures_iter(line_text) {
                let fn_name = &cap[1];
                // Skip common non-view request handlers (test utilities, etc.).
                if fn_name.starts_with("test_") {
                    continue;
                }
                let result = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, protocol, confidence
                     ) VALUES (
                        ?1, ?2, ?3, 'python',
                        ?1, ?2, ?3, 'python',
                        'django_view', 'http', 0.85
                     )",
                    rusqlite::params![file_id, line_no, fn_name],
                );
                match result {
                    Ok(n) if n > 0 => view_count += 1,
                    Ok(_) => {}
                    Err(e) => debug!(err = %e, fn_name = fn_name, "Failed to insert django_view (fbv)"),
                }
            }
        }
    }

    Ok(view_count)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run all Django detection passes and write results to the database.
///
/// Non-fatal: individual detection failures are logged as warnings and the
/// connector proceeds.
pub fn connect(db: &Database, project_root: &Path) -> Result<()> {
    let conn = db.conn();

    let model_count = detect_django_models(conn, project_root)
        .context("Django model detection failed")?;
    info!(model_count, "Django models detected");

    let route_count = detect_django_urls(conn, project_root)
        .context("Django URL detection failed")?;
    info!(route_count, "Django URL patterns detected");

    let view_count = detect_django_views(conn, project_root)
        .context("Django view detection failed")?;
    info!(view_count, "Django views detected");

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "django_tests.rs"]
mod tests;
