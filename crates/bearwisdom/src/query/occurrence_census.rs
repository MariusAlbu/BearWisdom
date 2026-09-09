//! Occurrence-level coverage, deliberately separate from edge-weighted metrics.
//!
//! The input is extracted references, NOT all references present in source.
//! Extraction recall and binding correctness require an independent oracle.

use std::collections::BTreeMap;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::occurrence::{OccurrenceBucket, OccurrenceCounts};
use crate::query::{stats::GENERATED_FILE_MATCH, QueryResult};
use crate::types::EdgeKind;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OccurrenceCensus {
    pub internal_files: u64,
    pub measured_files: u64,
    pub missing_files: u64,
    /// Source-content mismatch, not a proof of semantic dependency freshness.
    pub stale_files: u64,
    /// All current measured references, including samples and generated files.
    pub raw: OccurrenceCounts,
    /// Excludes samples, document imports and generated source symmetrically.
    pub code: OccurrenceCounts,
    pub excluded_snippets: u64,
    pub excluded_document_imports: u64,
    pub excluded_generated: u64,
    pub by_language_kind: BTreeMap<String, OccurrenceCounts>,
    /// Null if any file is unmeasured/stale or the eligible denominator is empty.
    /// Missing source attribution and unsupported profiles stay in the denominator.
    pub binding_coverage_percent: Option<f64>,
    /// Always null until independently labelled ground truth is supplied.
    pub binding_precision_percent: Option<f64>,
    pub correct_binding_recall_percent: Option<f64>,
}

/// Read-only, uncached census. Historical databases without the census table
/// remain readable; all their internal files are reported as unmeasured.
pub fn occurrence_census(db: &Database) -> QueryResult<OccurrenceCensus> {
    let _timer = db.timer("occurrence_census");
    let conn = db.conn();
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'resolution_census')",
        [], |row| row.get(0),
    )?;
    if !exists {
        let internal_files = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'internal'",
            [],
            |row| row.get(0),
        )?;
        return Ok(OccurrenceCensus {
            internal_files,
            missing_files: internal_files,
            ..Default::default()
        });
    }
    // One statement gives a consistent SQLite read snapshot even during indexing.
    let mut statement = conn.prepare(&format!(
        "SELECT f.hash, c.content_hash, c.counts_json, ({GENERATED_FILE_MATCH})
         FROM files f LEFT JOIN resolution_census c ON c.file_id = f.id
         WHERE f.origin = 'internal'"
    ))?;
    let mut rows = statement.query([])?;
    let mut report = OccurrenceCensus::default();
    while let Some(row) = rows.next()? {
        report.internal_files += 1;
        let hash: String = row.get(0)?;
        let measured_hash: Option<String> = row.get(1)?;
        match measured_hash {
            None => {
                report.missing_files += 1;
                continue;
            }
            Some(measured) if measured != hash => {
                report.stale_files += 1;
                continue;
            }
            Some(_) => report.measured_files += 1,
        }
        let json: String = row.get(2)?;
        let buckets: Vec<OccurrenceBucket> =
            serde_json::from_str(&json).context("Invalid persisted occurrence census")?;
        let generated: bool = row.get(3)?;
        for bucket in buckets {
            report.raw.add(bucket.disposition, bucket.count);
            // Disjoint exclusion buckets, applied equally to every outcome.
            if generated {
                report.excluded_generated += bucket.count;
            } else if bucket.from_snippet {
                report.excluded_snippets += bucket.count;
            } else if matches!(bucket.language.as_str(), "markdown" | "mdx")
                && bucket.kind == EdgeKind::Imports
            {
                report.excluded_document_imports += bucket.count;
            } else {
                report.code.add(bucket.disposition, bucket.count);
                let kind: &'static str = bucket.kind.into();
                report
                    .by_language_kind
                    .entry(format!("{}.{}", bucket.language, kind))
                    .or_default()
                    .add(bucket.disposition, bucket.count);
            }
        }
    }
    if report.missing_files == 0 && report.stale_files == 0 {
        report.binding_coverage_percent = report.code.binding_coverage_percent();
    }
    Ok(report)
}

#[cfg(test)]
#[path = "occurrence_census_tests.rs"]
mod tests;
