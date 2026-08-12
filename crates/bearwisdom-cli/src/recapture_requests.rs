// =============================================================================
// recapture_requests.rs — `--project` request accounting for the quality
// baseline commands.
//
// A scoped run asserts that every named project is capturable. Anything that
// blocks capture — a path that is gone, a root whose source tree was emptied,
// or a name matching no baseline entry — is recorded here so the run reports
// it instead of preserving the entry untouched and exiting clean.
// =============================================================================

use std::collections::BTreeSet;

/// Reason tag for a requested name that matches no entry in the baseline file.
pub(crate) const UNKNOWN_PROJECT: &str = "unknown_project";

/// Why a baseline entry produced no fresh metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipReason {
    /// The entry's `path` does not exist on disk.
    PathMissing,
    /// The root exists but holds no non-hidden entry: the source tree is gone
    /// and only cache directories such as `.bearwisdom/` remain. Indexing it
    /// would write zeroes across every metric.
    GhostSource,
}

impl SkipReason {
    /// Machine-readable tag carried in JSON output.
    pub(crate) fn tag(self) -> &'static str {
        match self {
            SkipReason::PathMissing => "path_missing",
            SkipReason::GhostSource => "ghost_source",
        }
    }

    /// Operator-facing explanation naming the on-disk state that blocks
    /// capture, so the reader knows what to restore.
    pub(crate) fn detail(self, path: &str) -> String {
        match self {
            SkipReason::PathMissing => format!("path not found: {path}"),
            SkipReason::GhostSource => format!(
                "source tree empty at {path} — only hidden entries remain, \
                 so there is nothing to index; restore the sources or drop \
                 the baseline entry"
            ),
        }
    }
}

/// A requested project that finished the run without fresh metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestFailure {
    pub(crate) project: String,
    pub(crate) reason: &'static str,
    pub(crate) detail: String,
}

impl RequestFailure {
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "project": self.project,
            "reason": self.reason,
            "detail": self.detail,
        })
    }
}

/// Render failures as the `failures` array of a command's JSON payload.
pub(crate) fn failures_json(failures: &[RequestFailure]) -> serde_json::Value {
    serde_json::Value::Array(failures.iter().map(RequestFailure::json).collect())
}

/// One line per failure, `project: detail`, for the error message.
pub(crate) fn failure_summary(failures: &[RequestFailure]) -> String {
    failures
        .iter()
        .map(|f| format!("  {} — {} ({})", f.project, f.detail, f.reason))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Tracks which `--project` names a run was scoped to and which of them
/// reached fresh metrics.
///
/// An unscoped run selects every baseline entry and records no failures: a
/// corpus sweep preserves missing and ghost entries by design, because the
/// caller named no project and may intend to restore sources later. A scoped
/// run records a failure for every requested name that was skipped or that
/// matched no baseline entry, so the caller cannot be told `ok` about a
/// project that was never captured.
pub(crate) struct RecaptureRequests {
    requested: BTreeSet<String>,
    matched: BTreeSet<String>,
    failures: Vec<RequestFailure>,
}

impl RecaptureRequests {
    pub(crate) fn new(only_projects: &[String]) -> Self {
        Self {
            requested: only_projects.iter().cloned().collect(),
            matched: BTreeSet::new(),
            failures: Vec::new(),
        }
    }

    /// True when the run was scoped to an explicit `--project` list.
    pub(crate) fn is_scoped(&self) -> bool {
        !self.requested.is_empty()
    }

    /// True when `name` is in scope for this run. Records the name as present
    /// in the baseline so it is not later reported as unknown.
    pub(crate) fn selects(&mut self, name: &str) -> bool {
        if !self.is_scoped() {
            return true;
        }
        if self.requested.contains(name) {
            self.matched.insert(name.to_string());
            return true;
        }
        false
    }

    /// Record that an in-scope project produced no fresh metrics. Only a
    /// scoped run treats this as a failure; an unscoped sweep tolerates it.
    pub(crate) fn record_skip(&mut self, name: &str, path: &str, reason: SkipReason) {
        if !self.is_scoped() {
            return;
        }
        self.failures.push(RequestFailure {
            project: name.to_string(),
            reason: reason.tag(),
            detail: reason.detail(path),
        });
    }

    /// Consume the tracker, appending one failure per requested name that
    /// matched no baseline entry. Failures are ordered by project name so the
    /// output is stable across runs.
    pub(crate) fn into_failures(self) -> Vec<RequestFailure> {
        let mut failures = self.failures;
        for name in self.requested.difference(&self.matched) {
            failures.push(RequestFailure {
                project: name.clone(),
                reason: UNKNOWN_PROJECT,
                detail: format!("no entry named '{name}' in the baseline file"),
            });
        }
        failures.sort_by(|a, b| a.project.cmp(&b.project));
        failures
    }
}

#[cfg(test)]
#[path = "recapture_requests_tests.rs"]
mod tests;
