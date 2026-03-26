// =============================================================================
// connectors/spring_di.rs  —  Spring DI connector
//
// Detects Spring dependency injection patterns in indexed Java files and
// creates `flow_edges` of type `di_binding`.
//
// Strategy:
//   1. Find all symbols annotated with @Service, @Component, or @Repository
//      by querying the `symbols` table for classes whose qualified_name is
//      known to the indexer as belonging to a stereotype concept, OR by
//      scanning Java files for the annotations directly.
//
//   Actually — we skip file I/O entirely here.  The existing `spring.rs`
//   connector already writes stereotype-annotated classes to the
//   `concept_members` table under the "spring-services" / "spring-repositories"
//   / "spring-components" concepts.  And the tree-sitter Java extractor
//   already emits `implements` edges in the `edges` table.
//
//   So this connector's job is purely a DB query:
//     - Collect symbol IDs of all Spring stereotype classes.
//     - For each, walk the `edges WHERE kind = 'implements'` to find the
//       interface they implement.
//     - Insert a `flow_edges` row (edge_type = 'di_binding') linking
//       interface → implementation.
//
//   This avoids re-parsing files and leverages the already-built graph.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan the symbol graph for Spring DI bindings and write `flow_edges`.
///
/// `project_root` is accepted to match the standard connector signature but is
/// only used as a fallback when the DB does not yet contain stereotype concept
/// members (e.g. the `spring.rs` connector has not been run yet).  In the
/// normal pipeline the graph is fully populated before this connector runs.
///
/// Returns the number of `flow_edges` rows inserted.
pub fn connect(conn: &Connection, project_root: &Path) -> Result<u32> {
    // Step 1: collect implementation class symbol IDs from stereotype concepts.
    let impl_ids = collect_stereotype_symbol_ids(conn, project_root)?;

    if impl_ids.is_empty() {
        debug!("No Spring stereotype classes found — skipping DI binding pass");
        info!(created = 0, "Spring DI connector: no bindings to create");
        return Ok(0);
    }

    // Step 2: for each impl, follow `implements` edges → interface symbols.
    let bindings = collect_interface_bindings(conn, &impl_ids)?;

    // Step 3: insert flow_edges.
    let created = insert_flow_edges(conn, &bindings)?;

    info!(created, "Spring DI connector: flow_edges inserted");
    Ok(created)
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// One resolved interface→implementation binding ready for insertion.
#[derive(Debug, Clone)]
struct DiBinding {
    /// `symbols.id` of the implementation class.
    impl_symbol_id: i64,
    /// `symbols.id` of the interface it satisfies.
    iface_symbol_id: i64,
    /// `files.id` of the implementation class.
    impl_file_id: i64,
    /// `files.id` of the interface.
    iface_file_id: i64,
    /// 1-based line of the implements relationship in the impl file.
    impl_line: Option<i64>,
    /// Simple name of the implementation class.
    impl_name: String,
    /// Simple name of the interface.
    iface_name: String,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Returns the symbol IDs of all classes with a Spring stereotype annotation.
///
/// Primary path: concept_members rows for the four Spring concepts.
/// Fallback path: regex scan of Java source files (for projects where
/// `register_spring_patterns` has not been run yet).
fn collect_stereotype_symbol_ids(conn: &Connection, project_root: &Path) -> Result<Vec<i64>> {
    // Try concept_members first — zero file I/O.
    let from_concepts = query_stereotype_concept_members(conn)?;
    if !from_concepts.is_empty() {
        debug!(
            count = from_concepts.len(),
            "Stereotype symbol IDs from concept_members"
        );
        return Ok(from_concepts);
    }

    debug!("No Spring concept members found — falling back to source scan");
    scan_stereotype_symbol_ids(conn, project_root)
}

/// Query `concept_members` for Spring stereotype concepts.
fn query_stereotype_concept_members(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt = conn
        .prepare(
            "SELECT cm.symbol_id
             FROM concept_members cm
             JOIN concepts c ON c.id = cm.concept_id
             WHERE c.name IN (
                 'spring-services',
                 'spring-repositories',
                 'spring-components',
                 'spring-controllers'
             )",
        )
        .context("Failed to prepare stereotype concept query")?;

    let ids: Vec<i64> = stmt
        .query_map([], |row| row.get(0))
        .context("Failed to query stereotype concept members")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect stereotype concept member rows")?;

    Ok(ids)
}

/// Fallback: scan Java source files for @Service/@Component/@Repository and
/// look up the class symbol in the DB.
fn scan_stereotype_symbol_ids(conn: &Connection, project_root: &Path) -> Result<Vec<i64>> {
    let re_stereotype = build_stereotype_annotation_regex();
    let re_class = build_class_decl_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'java'")
        .context("Failed to prepare Java files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Java files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Java file rows")?;

    let mut ids: Vec<i64> = Vec::new();

    for (_file_id, rel_path) in &files {
        let abs_path = project_root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Java file");
                continue;
            }
        };

        scan_stereotypes_in_source(conn, &source, rel_path, &re_stereotype, &re_class, &mut ids);
    }

    debug!(count = ids.len(), "Stereotype symbol IDs from source scan");
    Ok(ids)
}

fn scan_stereotypes_in_source(
    conn: &Connection,
    source: &str,
    rel_path: &str,
    re_stereotype: &Regex,
    re_class: &Regex,
    out: &mut Vec<i64>,
) {
    let lines: Vec<&str> = source.lines().collect();
    let mut pending = false;

    for (idx, line) in lines.iter().enumerate() {
        if re_stereotype.is_match(line) {
            pending = true;
            continue;
        }

        if pending {
            // Skip blank lines and stacked annotations.
            if line.trim().is_empty() || line.trim_start().starts_with('@') {
                continue;
            }

            if let Some(cap) = re_class.captures(line) {
                let class_name = &cap[1];
                let symbol_id: Option<i64> = conn
                    .query_row(
                        "SELECT s.id FROM symbols s
                         JOIN files f ON f.id = s.file_id
                         WHERE s.name = ?1 AND f.path = ?2 AND s.kind = 'class'
                         LIMIT 1",
                        rusqlite::params![class_name, rel_path],
                        |r| r.get(0),
                    )
                    .optional();

                if let Some(sid) = symbol_id {
                    out.push(sid);
                } else {
                    debug!(
                        class = %class_name,
                        "Spring stereotype class not indexed — skipping"
                    );
                }
            }

            pending = false;
            // If the line was not a class declaration the annotation was a
            // false positive (e.g. on an enum) — silently discard.
            let _ = idx;
        }
    }
}

/// For each implementation symbol ID, find the interfaces it implements via
/// existing `edges WHERE kind = 'implements'` rows.
fn collect_interface_bindings(conn: &Connection, impl_ids: &[i64]) -> Result<Vec<DiBinding>> {
    let mut bindings: Vec<DiBinding> = Vec::new();

    for &impl_id in impl_ids {
        // `edges` source_id = impl, target_id = interface (convention from the
        // tree-sitter Java extractor: "FooService implements IFooService" →
        // edge(source=FooService, target=IFooService, kind='implements')).
        let mut stmt = conn
            .prepare(
                "SELECT
                     e.target_id,
                     si.file_id,
                     sf.file_id,
                     e.source_line,
                     si.name,
                     sf.name
                 FROM edges e
                 JOIN symbols si ON si.id = e.source_id
                 JOIN symbols sf ON sf.id = e.target_id
                 WHERE e.source_id = ?1 AND e.kind = 'implements'",
            )
            .context("Failed to prepare implements edges query")?;

        let rows = stmt
            .query_map([impl_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,   // iface_symbol_id
                    row.get::<_, i64>(1)?,   // impl_file_id
                    row.get::<_, i64>(2)?,   // iface_file_id
                    row.get::<_, Option<i64>>(3)?, // impl_line
                    row.get::<_, String>(4)?,  // impl_name
                    row.get::<_, String>(5)?,  // iface_name
                ))
            })
            .context("Failed to query implements edges")?;

        for row in rows {
            let (iface_symbol_id, impl_file_id, iface_file_id, impl_line, impl_name, iface_name) =
                row.context("Failed to read implements edge row")?;

            bindings.push(DiBinding {
                impl_symbol_id: impl_id,
                iface_symbol_id,
                impl_file_id,
                iface_file_id,
                impl_line,
                impl_name,
                iface_name,
            });
        }
    }

    debug!(count = bindings.len(), "DI bindings resolved from implements edges");
    Ok(bindings)
}

/// Insert `flow_edges` rows for each binding.  Returns the insert count.
///
/// `flow_edges` has no UNIQUE constraint, so deduplication is done with an
/// explicit existence check before each insert.
fn insert_flow_edges(conn: &Connection, bindings: &[DiBinding]) -> Result<u32> {
    let mut created: u32 = 0;

    for b in bindings {
        // Guard: skip if an identical binding already exists.
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM flow_edges
                 WHERE source_file_id = ?1
                   AND source_symbol  = ?2
                   AND target_file_id = ?3
                   AND target_symbol  = ?4
                   AND edge_type      = 'di_binding'
                 LIMIT 1",
                rusqlite::params![b.impl_file_id, b.impl_name, b.iface_file_id, b.iface_name],
                |_| Ok(true),
            )
            .optional()
            .unwrap_or(false);

        if exists {
            continue;
        }

        let metadata = serde_json::json!({
            "source": "spring_di",
            "impl_symbol_id": b.impl_symbol_id,
            "iface_symbol_id": b.iface_symbol_id,
        })
        .to_string();

        let result = conn.execute(
            "INSERT INTO flow_edges (
                 source_file_id, source_line, source_symbol, source_language,
                 target_file_id, target_line, target_symbol, target_language,
                 edge_type, confidence, metadata
             ) VALUES (
                 ?1, ?2, ?3, 'java',
                 ?4, NULL, ?5, 'java',
                 'di_binding', 0.85, ?6
             )",
            rusqlite::params![
                b.impl_file_id,
                b.impl_line,
                b.impl_name,
                b.iface_file_id,
                b.iface_name,
                metadata,
            ],
        );

        match result {
            Ok(n) if n > 0 => created += 1,
            Ok(_) => {}
            Err(e) => {
                debug!(
                    impl_name = %b.impl_name,
                    iface_name = %b.iface_name,
                    err = %e,
                    "Failed to insert DI binding flow_edge"
                );
            }
        }
    }

    Ok(created)
}

// ---------------------------------------------------------------------------
// Regex builders (used by the fallback source scan)
// ---------------------------------------------------------------------------

fn build_stereotype_annotation_regex() -> Regex {
    Regex::new(r"@(Service|Component|Repository)\b")
        .expect("stereotype annotation regex is valid")
}

fn build_class_decl_regex() -> Regex {
    Regex::new(r"\bclass\s+(\w+)").expect("class decl regex is valid")
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
#[path = "spring_di_tests.rs"]
mod tests;
