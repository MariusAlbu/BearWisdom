// =============================================================================
// query/references.rs  —  find-all-references query
//
// "Who uses this symbol?"
//
// Returns all edges that point TO the given symbol (incoming edges).
// For each edge we return the referencing symbol name, file, line, and
// edge kind so the caller can display a proper reference list.
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use crate::types::ReferenceResult;
use anyhow::Context;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// How the resolved part of a reference result was recovered.
///
/// `RefResolutionLog` preserves individual source sites.  `LegacyEdges` is
/// retained for indexes produced before that log existed; an edge is an
/// aggregate relationship and can lose repeated same-line occurrences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedReferenceSource {
    RefResolutionLog,
    Mixed,
    LegacyEdges,
}

/// Whether the persisted per-file resolver census covers the current files.
/// This is coverage of what the resolver observed, not a claim that every
/// lexical occurrence in a language was extracted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceCoverage {
    Complete,
    Partial,
    Unknown,
}

/// A source-attested site that could not be connected to the selected
/// declaration identity.  A matching name is useful evidence, but is never
/// presented as a resolved reference when namesakes exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceAttestedOccurrence {
    pub referencing_symbol: String,
    pub referencing_kind: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub target_name: String,
    pub edge_kind: String,
    /// `unresolved` or `drained` as recorded by the resolver.
    pub outcome: String,
    /// Number of internal declarations with this exact emitted target name.
    /// More than one means this occurrence is explicitly name-only evidence.
    pub candidate_declaration_count: u32,
    /// `ref_resolution_log` preserves source columns; `legacy_unresolved_ref`
    /// is an older, line-only diagnostic record.
    pub evidence_source: String,
}

/// Metadata that prevents an empty resolved list from being read as proof
/// that no source occurrence exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceCoverage {
    pub occurrence_coverage: OccurrenceCoverage,
    pub internal_files: u64,
    pub measured_files: u64,
    pub missing_files: u64,
    pub stale_files: u64,
    pub resolved_source: ResolvedReferenceSource,
    pub resolved_total: u64,
    pub resolved_truncated: bool,
    pub source_attested_total: u64,
    pub source_attested_truncated: bool,
}

/// Evidence for one already-selected declaration ID.  Resolved edges and
/// source-attested unresolved occurrences deliberately live in separate
/// fields: a spelling match never upgrades an unresolved site into a graph
/// fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceEvidence {
    pub resolved: Vec<ReferenceResult>,
    pub source_attested_unresolved: Vec<SourceAttestedOccurrence>,
    pub coverage: ReferenceCoverage,
}

fn cache_key(target_name: &str, limit: usize) -> String {
    format!("{target_name}\u{1f}{limit}")
}

/// Find all symbols that reference `target_name`.
///
/// `target_name` may be a simple name or a fully qualified name.
/// When it is a simple name, all symbols with that name are searched
/// (returns references to all overloads / same-named symbols).
///
/// `limit`: maximum number of results (0 = unlimited).
pub fn find_references(
    db: &Database,
    target_name: &str,
    limit: usize,
) -> QueryResult<Vec<ReferenceResult>> {
    let _timer = db.timer("find_references");
    let cache_key = cache_key(target_name, limit);

    // Check cache first.
    if let Some(ref cache) = db.query_cache {
        if let Some(cached) = cache.get_references(&cache_key) {
            if let Ok(result) = serde_json::from_str::<Vec<ReferenceResult>>(&cached) {
                return Ok(result);
            }
        }
    }

    let conn = db.conn();

    // Resolve exact qualified identities first, regardless of the language's
    // qualified-name separator (`.`, `::`, `\\`, or something else). Only
    // fall back to the potentially ambiguous simple name when no exact
    // qualified identity exists.
    let target_ids: Vec<i64> = {
        let mut qualified = conn
            .prepare(
                "SELECT id FROM symbols
                 WHERE qualified_name = ?1 AND name <> ?1 AND origin = 'internal'",
            )
            .context("Failed to prepare qualified target lookup")?;
        let rows = qualified
            .query_map([target_name], |r| r.get(0))
            .context("Failed to query qualified target")?;
        let exact = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect qualified target ids")?;
        if !exact.is_empty() {
            exact
        } else {
            let mut simple = conn
                .prepare("SELECT id FROM symbols WHERE name = ?1 AND origin = 'internal'")
                .context("Failed to prepare simple target lookup")?;
            let rows = simple
                .query_map([target_name], |r| r.get(0))
                .context("Failed to query simple target")?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect simple target ids")?
        }
    };

    if target_ids.is_empty() {
        return Ok(vec![]);
    }

    let (mut results, _) = resolved_for_ids(conn, &target_ids)?;
    sort_and_limit(&mut results, limit);

    // Store in cache.
    if let Some(ref cache) = db.query_cache {
        if let Ok(json) = serde_json::to_string(&results) {
            cache.put_references(cache_key, json);
        }
    }

    Ok(results)
}

/// Retrieve evidence for one exact declaration selected by its database ID.
///
/// The caller supplies the declaration's emitted name solely to find resolver
/// failures that could not carry a target ID.  This module never interprets
/// qualified-name separators or infers a declaration from a name match.
pub fn evidence_for_declaration(
    db: &Database,
    declaration_id: i64,
    declaration_name: &str,
    limit: usize,
) -> QueryResult<ReferenceEvidence> {
    let _timer = db.timer("reference_evidence");
    let conn = db.conn();
    let coverage = occurrence_coverage(conn)?;
    let evidence_mode = resolution_evidence_mode(conn, coverage.occurrence_coverage)?;

    let (mut resolved, resolved_source) = match evidence_mode {
        ResolutionEvidenceMode::Log => resolved_from_log(conn, &[declaration_id])?,
        ResolutionEvidenceMode::Mixed => resolved_from_mixed_sources(conn, &[declaration_id])?,
        ResolutionEvidenceMode::Legacy => resolved_from_edges(conn, &[declaration_id])?,
    };
    let resolved_total = resolved.len() as u64;
    sort_and_limit(&mut resolved, limit);

    let candidate_declaration_count: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name = ?1 AND origin = 'internal'",
            [declaration_name],
            |row| row.get(0),
        )
        .context("Failed to count declaration-name candidates")?;
    let mut unresolved = match evidence_mode {
        ResolutionEvidenceMode::Log => {
            unresolved_from_log(conn, declaration_name, candidate_declaration_count)?
        }
        ResolutionEvidenceMode::Mixed => {
            unresolved_from_mixed_sources(conn, declaration_name, candidate_declaration_count)?
        }
        ResolutionEvidenceMode::Legacy => {
            unresolved_from_legacy_table(conn, declaration_name, candidate_declaration_count)?
        }
    };
    let source_attested_total = unresolved.len() as u64;
    unresolved.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
    });
    if limit > 0 && unresolved.len() > limit {
        unresolved.truncate(limit);
    }

    Ok(ReferenceEvidence {
        resolved,
        source_attested_unresolved: unresolved,
        coverage: ReferenceCoverage {
            resolved_truncated: limit > 0 && resolved_total > limit as u64,
            source_attested_truncated: limit > 0 && source_attested_total > limit as u64,
            occurrence_coverage: coverage.occurrence_coverage,
            internal_files: coverage.internal_files,
            measured_files: coverage.measured_files,
            missing_files: coverage.missing_files,
            stale_files: coverage.stale_files,
            resolved_source,
            resolved_total,
            source_attested_total,
        },
    })
}

#[derive(Default)]
struct CoverageCounts {
    occurrence_coverage: OccurrenceCoverage,
    internal_files: u64,
    measured_files: u64,
    missing_files: u64,
    stale_files: u64,
}

impl Default for OccurrenceCoverage {
    fn default() -> Self {
        Self::Unknown
    }
}

fn occurrence_coverage(conn: &Connection) -> QueryResult<CoverageCounts> {
    let mut stmt = conn.prepare(
        "SELECT f.hash, c.content_hash
         FROM files f LEFT JOIN resolution_census c ON c.file_id = f.id
         WHERE f.origin = 'internal'",
    )?;
    let mut rows = stmt.query([])?;
    let mut counts = CoverageCounts::default();
    while let Some(row) = rows.next()? {
        counts.internal_files += 1;
        let current_hash: String = row.get(0)?;
        let measured_hash: Option<String> = row.get(1)?;
        match measured_hash {
            Some(hash) if hash == current_hash => counts.measured_files += 1,
            Some(_) => counts.stale_files += 1,
            None => counts.missing_files += 1,
        }
    }
    counts.occurrence_coverage = if counts.internal_files == 0 {
        OccurrenceCoverage::Unknown
    } else if counts.missing_files == 0 && counts.stale_files == 0 {
        OccurrenceCoverage::Complete
    } else if counts.measured_files > 0 {
        OccurrenceCoverage::Partial
    } else {
        OccurrenceCoverage::Unknown
    };
    Ok(counts)
}

#[derive(Clone, Copy)]
enum ResolutionEvidenceMode {
    Log,
    Mixed,
    Legacy,
}

fn resolution_evidence_mode(
    conn: &Connection,
    coverage: OccurrenceCoverage,
) -> QueryResult<ResolutionEvidenceMode> {
    if coverage == OccurrenceCoverage::Complete {
        return Ok(ResolutionEvidenceMode::Log);
    }
    let has_log: bool = conn
        .query_row("SELECT EXISTS(SELECT 1 FROM ref_resolutions)", [], |row| {
            row.get(0)
        })
        .context("Failed to check source-attested resolution log")?;
    Ok(if has_log {
        ResolutionEvidenceMode::Mixed
    } else {
        ResolutionEvidenceMode::Legacy
    })
}

fn resolved_for_ids(
    conn: &Connection,
    target_ids: &[i64],
) -> QueryResult<(Vec<ReferenceResult>, ResolvedReferenceSource)> {
    match resolution_evidence_mode(conn, occurrence_coverage(conn)?.occurrence_coverage)? {
        ResolutionEvidenceMode::Log => resolved_from_log(conn, target_ids),
        ResolutionEvidenceMode::Mixed => resolved_from_mixed_sources(conn, target_ids),
        ResolutionEvidenceMode::Legacy => resolved_from_edges(conn, target_ids),
    }
}

fn resolved_from_mixed_sources(
    conn: &Connection,
    target_ids: &[i64],
) -> QueryResult<(Vec<ReferenceResult>, ResolvedReferenceSource)> {
    use std::collections::HashSet;

    let (mut resolved, _) = resolved_from_log(conn, target_ids)?;
    let source_attested_keys = resolved.iter().map(reference_key).collect::<HashSet<_>>();
    let (legacy, _) = resolved_from_edges(conn, target_ids)?;
    resolved.extend(
        legacy
            .into_iter()
            .filter(|reference| !source_attested_keys.contains(&reference_key(reference))),
    );
    Ok((resolved, ResolvedReferenceSource::Mixed))
}

fn reference_key(reference: &ReferenceResult) -> (String, String, String, u32, String) {
    (
        reference.referencing_symbol.clone(),
        reference.referencing_kind.clone(),
        reference.file_path.clone(),
        reference.line,
        reference.edge_kind.clone(),
    )
}

fn resolved_from_log(
    conn: &Connection,
    target_ids: &[i64],
) -> QueryResult<(Vec<ReferenceResult>, ResolvedReferenceSource)> {
    let placeholders = placeholders(target_ids.len());
    let sql = format!(
        "SELECT src.name, src.kind, f.path, rr.source_line, rr.kind, rr.confidence
         FROM ref_resolutions rr
         JOIN symbols src ON rr.source_id = src.id
         JOIN files f ON src.file_id = f.id
         WHERE rr.outcome = 'resolved' AND rr.target_id IN ({placeholders})"
    );
    let mut stmt = conn
        .prepare(&sql)
        .context("Failed to prepare source-attested references")?;
    let rows = stmt.query_map(rusqlite::params_from_iter(target_ids.iter()), reference_row)?;
    let result = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((result, ResolvedReferenceSource::RefResolutionLog))
}

fn resolved_from_edges(
    conn: &Connection,
    target_ids: &[i64],
) -> QueryResult<(Vec<ReferenceResult>, ResolvedReferenceSource)> {
    let placeholders = placeholders(target_ids.len());
    let sql = format!(
        "SELECT src.name, src.kind, f.path, e.source_line, e.kind, e.confidence
         FROM edges e
         JOIN symbols src ON e.source_id = src.id
         JOIN files f ON src.file_id = f.id
         WHERE e.target_id IN ({placeholders})"
    );
    let mut stmt = conn
        .prepare(&sql)
        .context("Failed to prepare legacy graph references")?;
    let rows = stmt.query_map(rusqlite::params_from_iter(target_ids.iter()), reference_row)?;
    let result = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((result, ResolvedReferenceSource::LegacyEdges))
}

fn reference_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReferenceResult> {
    Ok(ReferenceResult {
        referencing_symbol: row.get(0)?,
        referencing_kind: row.get(1)?,
        file_path: row.get(2)?,
        line: row.get::<_, Option<u32>>(3)?.unwrap_or(0),
        edge_kind: row.get(4)?,
        confidence: row.get::<_, Option<f64>>(5)?.unwrap_or(0.0),
    })
}

fn unresolved_from_log(
    conn: &Connection,
    declaration_name: &str,
    candidate_declaration_count: u32,
) -> QueryResult<Vec<SourceAttestedOccurrence>> {
    let mut stmt = conn.prepare(
        "SELECT src.name, src.kind, f.path, rr.source_line, rr.source_col,
                rr.target_name, rr.kind, rr.outcome
         FROM ref_resolutions rr
         JOIN symbols src ON rr.source_id = src.id
         JOIN files f ON src.file_id = f.id
         WHERE rr.target_name = ?1 AND rr.outcome IN ('unresolved', 'drained')",
    )?;
    let rows = stmt.query_map([declaration_name], |row| {
        Ok(SourceAttestedOccurrence {
            referencing_symbol: row.get(0)?,
            referencing_kind: row.get(1)?,
            file_path: row.get(2)?,
            line: row.get::<_, Option<u32>>(3)?.unwrap_or(0),
            column: row.get::<_, Option<u32>>(4)?.unwrap_or(0),
            target_name: row.get(5)?,
            edge_kind: row.get(6)?,
            outcome: row.get(7)?,
            candidate_declaration_count,
            evidence_source: "ref_resolution_log".to_owned(),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn unresolved_from_mixed_sources(
    conn: &Connection,
    declaration_name: &str,
    candidate_declaration_count: u32,
) -> QueryResult<Vec<SourceAttestedOccurrence>> {
    use std::collections::HashSet;

    let mut occurrences = unresolved_from_log(conn, declaration_name, candidate_declaration_count)?;
    let source_attested_keys = occurrences
        .iter()
        .map(unresolved_key)
        .collect::<HashSet<_>>();
    let legacy = unresolved_from_legacy_table(conn, declaration_name, candidate_declaration_count)?;
    occurrences.extend(
        legacy
            .into_iter()
            .filter(|occurrence| !source_attested_keys.contains(&unresolved_key(occurrence))),
    );
    Ok(occurrences)
}

fn unresolved_key(
    occurrence: &SourceAttestedOccurrence,
) -> (String, String, String, u32, String, String) {
    (
        occurrence.referencing_symbol.clone(),
        occurrence.referencing_kind.clone(),
        occurrence.file_path.clone(),
        occurrence.line,
        occurrence.edge_kind.clone(),
        occurrence.outcome.clone(),
    )
}

fn unresolved_from_legacy_table(
    conn: &Connection,
    declaration_name: &str,
    candidate_declaration_count: u32,
) -> QueryResult<Vec<SourceAttestedOccurrence>> {
    let mut stmt = conn.prepare(
        "SELECT src.name, src.kind, f.path, ur.source_line, ur.target_name,
                ur.kind, ur.drained
         FROM unresolved_refs ur
         JOIN symbols src ON ur.source_id = src.id
         JOIN files f ON src.file_id = f.id
         WHERE ur.target_name = ?1",
    )?;
    let rows = stmt.query_map([declaration_name], |row| {
        let drained: bool = row.get(6)?;
        Ok(SourceAttestedOccurrence {
            referencing_symbol: row.get(0)?,
            referencing_kind: row.get(1)?,
            file_path: row.get(2)?,
            line: row.get::<_, Option<u32>>(3)?.unwrap_or(0),
            column: 0,
            target_name: row.get(4)?,
            edge_kind: row.get(5)?,
            outcome: if drained { "drained" } else { "unresolved" }.to_owned(),
            candidate_declaration_count,
            evidence_source: "legacy_unresolved_ref".to_owned(),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn placeholders(count: usize) -> String {
    std::iter::repeat("?")
        .take(count)
        .collect::<Vec<_>>()
        .join(",")
}

fn sort_and_limit(results: &mut Vec<ReferenceResult>, limit: usize) {
    results.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));
    if limit > 0 && results.len() > limit {
        results.truncate(limit);
    }
}

/// JSON-returning variant of [`find_references`] for use in MCP and CLI paths.
///
/// Returns the raw cached JSON string on a cache hit, skipping the
/// deserialize → struct → reserialize roundtrip that occurs when the caller
/// would otherwise call `find_references` and then `serde_json::to_string`.
pub fn find_references_json(
    db: &Database,
    target_name: &str,
    limit: usize,
) -> super::QueryResult<String> {
    let cache_key = cache_key(target_name, limit);
    // Raw cache hit: return JSON directly without deserializing.
    if let Some(ref cache) = db.query_cache {
        if let Some(raw) = cache.get_references_raw(&cache_key) {
            return Ok(raw);
        }
    }
    let result = find_references(db, target_name, limit)?;
    serde_json::to_string(&result)
        .map_err(|e| super::QueryError::Internal(anyhow::anyhow!("serialization error: {e}")))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "references_tests.rs"]
mod tests;
