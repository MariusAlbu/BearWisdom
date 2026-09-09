use std::collections::HashMap;

use super::*;
use anyhow::{ensure, Result};

pub fn evaluate(
    revision: CorpusRevision,
    expected: &[ExpectedReference],
    observations: &[Observation],
) -> Result<OracleReport> {
    let mut observed = HashMap::new();
    for observation in observations {
        ensure!(
            observed.insert(observation.site, observation).is_none(),
            "Duplicate observation at {:?}; disambiguate source spans, do not pick a target",
            observation.site
        );
    }
    let mut labels = HashMap::new();
    let mut counts = OracleCounts::default();
    let mut references = Vec::with_capacity(expected.len());
    for label in expected {
        ensure!(
            labels.insert(label.site, label).is_none(),
            "Duplicate ground-truth site: {:?}",
            label.site
        );
        let observation = observed.get(&label.site);
        let actual = observation.and_then(|o| o.binding);
        let verdict = match (observation, actual, label.target) {
            (None, _, _) => Verdict::NotExtracted,
            (Some(_), None, _) => Verdict::MissingResolution,
            (_, Some(ObservedBinding::Resolved(target)), Some(expected)) if target == expected => {
                Verdict::Correct
            }
            (_, Some(ObservedBinding::Resolved(_) | ObservedBinding::DanglingTarget), _) => {
                Verdict::Incorrect
            }
            (_, Some(ObservedBinding::Unresolved | ObservedBinding::Drained), None) => {
                Verdict::CorrectUnbound
            }
            _ => Verdict::Unresolved,
        };
        counts.labelled += 1;
        counts.expected_bound += u64::from(label.target.is_some());
        match verdict {
            Verdict::Correct => counts.correct += 1,
            Verdict::Incorrect => counts.incorrect += 1,
            Verdict::CorrectUnbound => counts.correct_unbound += 1,
            Verdict::Unresolved => counts.unresolved += 1,
            Verdict::NotExtracted => counts.not_extracted += 1,
            Verdict::MissingResolution => counts.missing_resolution += 1,
        }
        references.push(EvaluatedReference {
            expected: label.clone(),
            actual,
            verdict,
        });
    }
    counts.unlabelled_observations = observed
        .keys()
        .filter(|site| !labels.contains_key(site))
        .count() as u64;
    counts.binding_precision_percent = percent(counts.correct, counts.correct + counts.incorrect);
    counts.correct_binding_recall_percent = percent(counts.correct, counts.expected_bound);
    counts.extraction_coverage_percent =
        percent(counts.labelled - counts.not_extracted, counts.labelled);
    Ok(OracleReport {
        revision,
        counts,
        references,
    })
}

fn percent(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 * 100.0 / denominator as f64)
}

/// Compare per-site identity, not just totals. Labels and corpus revision must
/// be identical. Improvements never cancel a regression at a different site.
pub fn compare(before: &OracleReport, after: &OracleReport) -> Result<Vec<OracleChange>> {
    ensure!(
        before.revision == after.revision,
        "Cannot compare different corpus revisions"
    );
    let earlier = unique_entries(&before.references)?;
    let later = unique_entries(&after.references)?;
    ensure!(
        earlier.len() == later.len(),
        "Ground-truth population changed"
    );
    let mut changes = Vec::new();
    for previous in &before.references {
        let site = previous.expected.site;
        let current = later
            .get(&site)
            .ok_or_else(|| anyhow::anyhow!("Ground-truth site missing: {site:?}"))?;
        ensure!(
            previous.expected == current.expected,
            "Expected target changed at {site:?}"
        );
        if previous == *current {
            continue;
        }
        let retargeted = matches!((previous.actual, current.actual),
            (Some(ObservedBinding::Resolved(a)), Some(ObservedBinding::Resolved(b))) if a != b);
        let regressed = (previous.verdict != current.verdict
            && matches!(previous.verdict, Verdict::Correct | Verdict::CorrectUnbound))
            || (current.verdict == Verdict::Incorrect && previous.verdict != Verdict::Incorrect)
            || (current.verdict == Verdict::NotExtracted
                && previous.verdict != Verdict::NotExtracted);
        changes.push(OracleChange {
            site,
            before: previous.clone(),
            after: (*current).clone(),
            retargeted,
            regressed,
        });
    }
    Ok(changes)
}

fn unique_entries(
    entries: &[EvaluatedReference],
) -> Result<HashMap<ReferenceSite, &EvaluatedReference>> {
    let mut map = HashMap::new();
    for entry in entries {
        ensure!(
            map.insert(entry.expected.site, entry).is_none(),
            "Duplicate evaluated reference"
        );
    }
    Ok(map)
}

#[cfg(test)]
#[path = "evaluate_tests.rs"]
mod tests;
