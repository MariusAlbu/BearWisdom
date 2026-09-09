//! Shared gate payload: legacy graph trends alongside occurrence evidence.

use anyhow::{Context, Result};
use bearwisdom::query::dead_code::{find_dead_code, DeadCodeOptions};
use bearwisdom::Database;

pub(super) fn build(db: &Database) -> Result<serde_json::Value> {
    let breakdown = bearwisdom::resolution_breakdown(db).context("resolution_breakdown failed")?;
    let dead = find_dead_code(
        db,
        &DeadCodeOptions {
            max_results: 0,
            ..Default::default()
        },
    )
    .context("find_dead_code failed")?;
    let occurrences = bearwisdom::query::occurrence_census::occurrence_census(db)
        .context("occurrence_census failed")?;
    Ok(serde_json::json!({
        "breakdown": breakdown,
        "health": dead.resolution_health,
        "occurrences": occurrences,
        "measurement_contract": {
            "breakdown": "legacy_edge_weighted_coverage_not_precision",
            "occurrences": "extracted_reference_binding_coverage_not_correctness",
            "correctness": "requires_independent_ground_truth",
            "extraction_recall": "not_measured",
            "freshness": "source_content_only_not_semantic_dependency_parity"
        }
    }))
}

#[cfg(test)]
#[path = "resolution_gate_report_tests.rs"]
mod tests;
