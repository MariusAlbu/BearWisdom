// =============================================================================
// drain_audit_report — stderr surfacing for quality-check drain-audit findings
// =============================================================================

use bearwisdom::query::drain_audit::DrainAuditFinding;

/// Print the drained-name findings for one project to stderr, capped at the
/// five biggest groups. Silent when the audit is clean.
pub fn print_findings(findings: &[DrainAuditFinding]) {
    if findings.is_empty() {
        return;
    }
    eprintln!(
        "  DRAIN-AUDIT: {} drained name(s) mask kind-compatible declarations",
        findings.len()
    );
    for f in findings.iter().take(5) {
        let example = f
            .matches
            .first()
            .map(|m| format!("{} ({}, {})", m.qualified_name, m.symbol_kind, m.origin))
            .unwrap_or_default();
        eprintln!(
            "    {}.{} '{}' drained {}x, {} declaration(s), e.g. {}",
            f.language, f.kind, f.target_name, f.drained_count, f.total_matches, example
        );
    }
}

/// Build the sorted per-class pooled rate report:
/// `(class, edges, unresolved, rate)`. Rate is
/// `edges / (edges + unresolved) * 100`, two decimals, 100.0 for an empty
/// group.
pub fn corpus_class_report(
    groups: &std::collections::BTreeMap<String, (i64, i64)>,
) -> Vec<(String, i64, i64, f64)> {
    groups
        .iter()
        .map(|(class, (edges, unresolved))| {
            let denom = edges + unresolved;
            let rate = if denom == 0 {
                100.0
            } else {
                (*edges as f64) * 100.0 / (denom as f64)
            };
            let rate = (rate * 100.0).round() / 100.0;
            (class.clone(), *edges, *unresolved, rate)
        })
        .collect()
}
