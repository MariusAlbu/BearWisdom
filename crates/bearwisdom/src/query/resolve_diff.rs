// =============================================================================
// query/resolve_diff.rs — flip-diff between two ref-resolution snapshots
//
// Compares an OLD `RefSnapshotEntry` set (loaded from a JSONL file produced by
// `query::ref_snapshot::write_snapshot_jsonl`) against a NEW set (the current
// DB state) and classifies every ref site whose key appears in both sets but
// whose outcome differs. Ref sites present in only one snapshot (a ref added
// or removed by a source edit) are out of scope — this instrument measures
// resolution-behavior drift on a fixed ref-site population, not code churn.
//
// Classification, in priority order, for a key present in both snapshots:
//   1. both resolved, different target qname -> retargeted
//   2. either side "drained"                 -> drain_transitions
//   3. old unresolved, new resolved          -> newly_resolved
//   4. old resolved, new unresolved          -> newly_unresolved
//   5. otherwise (identical outcome)         -> not reported
// =============================================================================

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::query::ref_snapshot::{export_ref_snapshot, RefSnapshotEntry};
use crate::query::QueryResult;

/// One flipped ref site, carrying both snapshots' outcome for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlipSample {
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub target: String,
    pub kind: String,
    pub old_outcome: String,
    pub new_outcome: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub old_target_qname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub new_target_qname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub old_target_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub new_target_file: Option<String>,
}

/// A classification bucket: total count plus a capped sample for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlipBucket {
    pub count: usize,
    pub samples: Vec<FlipSample>,
}

impl FlipBucket {
    fn from_all(mut all: Vec<FlipSample>, cap: usize) -> Self {
        let count = all.len();
        all.truncate(cap);
        Self { count, samples: all }
    }
}

/// The result of diffing two ref-resolution snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveDiffReport {
    pub old_ref_count: usize,
    pub new_ref_count: usize,
    /// Ref sites whose key is present in both snapshots — the population the
    /// classification buckets are drawn from.
    pub compared_ref_count: usize,
    pub newly_resolved: FlipBucket,
    pub newly_unresolved: FlipBucket,
    pub retargeted: FlipBucket,
    pub drain_transitions: FlipBucket,
}

/// Diff the current DB state against a previously-captured snapshot.
/// `sample_cap` bounds how many example rows each bucket carries (pass
/// `usize::MAX` for the uncapped `--full` view).
pub fn diff_against_db(
    db: &Database,
    old: &[RefSnapshotEntry],
    sample_cap: usize,
) -> QueryResult<ResolveDiffReport> {
    let new = export_ref_snapshot(db)?;
    Ok(diff_snapshots(old, &new, sample_cap))
}

/// Pure diff over two already-loaded snapshots. Deterministic: buckets are
/// sorted by key before truncation so the sample is stable across runs.
pub fn diff_snapshots(
    old: &[RefSnapshotEntry],
    new: &[RefSnapshotEntry],
    sample_cap: usize,
) -> ResolveDiffReport {
    let old_by_key: FxHashMap<_, _> = old.iter().map(|e| (e.key(), e)).collect();
    let new_by_key: FxHashMap<_, _> = new.iter().map(|e| (e.key(), e)).collect();

    let mut newly_resolved = Vec::new();
    let mut newly_unresolved = Vec::new();
    let mut retargeted = Vec::new();
    let mut drain_transitions = Vec::new();

    let mut compared_keys: Vec<_> = old_by_key.keys().filter(|k| new_by_key.contains_key(*k)).collect();
    compared_keys.sort_unstable();

    for key in &compared_keys {
        let o = old_by_key[key];
        let n = new_by_key[key];
        if o.outcome == n.outcome && o.target_qname == n.target_qname {
            continue;
        }
        let sample = || FlipSample {
            file: n.file.clone(),
            line: n.line,
            col: n.col,
            target: n.target.clone(),
            kind: n.kind.clone(),
            old_outcome: o.outcome.clone(),
            new_outcome: n.outcome.clone(),
            old_target_qname: o.target_qname.clone(),
            new_target_qname: n.target_qname.clone(),
            old_target_file: o.target_file.clone(),
            new_target_file: n.target_file.clone(),
        };
        if o.outcome == "resolved" && n.outcome == "resolved" {
            retargeted.push(sample());
        } else if o.outcome == "drained" || n.outcome == "drained" {
            drain_transitions.push(sample());
        } else if o.outcome != "resolved" && n.outcome == "resolved" {
            newly_resolved.push(sample());
        } else if o.outcome == "resolved" && n.outcome != "resolved" {
            newly_unresolved.push(sample());
        }
    }

    ResolveDiffReport {
        old_ref_count: old.len(),
        new_ref_count: new.len(),
        compared_ref_count: compared_keys.len(),
        newly_resolved: FlipBucket::from_all(newly_resolved, sample_cap),
        newly_unresolved: FlipBucket::from_all(newly_unresolved, sample_cap),
        retargeted: FlipBucket::from_all(retargeted, sample_cap),
        drain_transitions: FlipBucket::from_all(drain_transitions, sample_cap),
    }
}

#[cfg(test)]
#[path = "resolve_diff_tests.rs"]
mod tests;
