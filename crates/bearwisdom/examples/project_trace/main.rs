//! Diagnose a frozen compiler cohort without modifying its source or baseline.
use anyhow::{ensure, Context, Result};
use bearwisdom::indexer::resolve::engine::trace;
use bearwisdom::resolution_oracle::project::{
    evaluate_manifest_with_mode, ProjectBindingMode, ProjectCall, ProjectManifest,
};
use bearwisdom::resolution_oracle::{compare, CorpusRevision, OracleReport, Verdict};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

#[derive(Deserialize)]
struct Baseline {
    fresh: OracleReport,
    #[serde(default)]
    binding_mode: Option<String>,
}

fn binding_mode(baseline: &Baseline) -> Result<ProjectBindingMode> {
    match baseline.binding_mode.as_deref() {
        None | Some("legacy") => Ok(ProjectBindingMode::Legacy),
        Some("configured_program") => Ok(ProjectBindingMode::ConfiguredProgram),
        Some(other) => anyhow::bail!("Unsupported baseline binding mode: {other}"),
    }
}

fn validate_labels(
    calls: &[ProjectCall],
    revision: CorpusRevision,
    baseline: &OracleReport,
) -> Result<()> {
    ensure!(
        revision == baseline.revision,
        "Baseline belongs to a different manifest revision"
    );
    let labels: HashMap<_, _> = calls
        .iter()
        .filter_map(|c| c.target.map(|t| (c.site, t)))
        .collect();
    ensure!(
        labels.len() == baseline.references.len(),
        "Baseline population differs from manifest"
    );
    let mut seen = HashSet::new();
    for reference in &baseline.references {
        let expected = &reference.expected;
        ensure!(seen.insert(expected.site), "Duplicate baseline occurrence");
        ensure!(
            labels.get(&expected.site).copied() == expected.target && expected.target.is_some(),
            "Baseline target differs from compiler manifest at {:?}",
            expected.site
        );
    }
    Ok(())
}

fn site_line(source: &str, offset: u32) -> Result<u32> {
    let offset = offset as usize;
    ensure!(
        offset < source.len() && source.is_char_boundary(offset),
        "Invalid UTF-8 selector offset"
    );
    Ok(source.as_bytes()[..offset]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32)
}

struct TraceGuard;
impl Drop for TraceGuard {
    fn drop(&mut self) {
        trace::deactivate();
        trace::clear_filter();
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let manifest_path = args
        .next()
        .context("Usage: project_trace <manifest.json> <baseline.json> <new-diagnostic.json>")?;
    let baseline_path = args.next().context("Missing baseline report")?;
    let output_path = args.next().context("Missing new diagnostic report")?;
    ensure!(args.next().is_none(), "Unexpected arguments");
    ensure!(
        !Path::new(&output_path).exists(),
        "Refusing to overwrite an existing report"
    );
    let manifest_bytes = std::fs::read(&manifest_path)?;
    let manifest: ProjectManifest = serde_json::from_slice(&manifest_bytes)?;
    manifest.verify_inputs()?;
    let baseline_bytes = std::fs::read(&baseline_path)?;
    let baseline: Baseline = serde_json::from_slice(&baseline_bytes)?;
    let mode = binding_mode(&baseline)?;
    validate_labels(
        &manifest.calls,
        CorpusRevision(Sha256::digest(&manifest_bytes).into()),
        &baseline.fresh,
    )?;
    let files: HashMap<_, _> = manifest.files.iter().map(|f| (f.id, f)).collect();
    let mut sources = HashMap::new();
    let mut filters = HashSet::new();
    let mut requested = Vec::new();
    for reference in baseline
        .fresh
        .references
        .iter()
        .filter(|r| r.verdict == Verdict::Unresolved)
    {
        let site = reference.expected.site;
        let file = files[&site.file];
        let source = match sources.entry(site.file) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(std::fs::read_to_string(&file.path)?)
            }
        };
        let line = site_line(source, site.byte_offset)?;
        filters.insert((file.index_path.clone(), line, String::new()));
        requested
            .push(serde_json::json!({"site":site,"file":file.index_path,"selector_line":line}));
    }
    ensure!(
        !requested.is_empty(),
        "Baseline has no unresolved occurrences"
    );
    trace::drain_collected();
    trace::set_filters(filters.into_iter().collect());
    trace::activate();
    let guard = TraceGuard;
    let report = evaluate_manifest_with_mode(Path::new(&manifest_path), mode)?;
    drop(guard);
    let traces: Vec<_> = trace::drain_collected().into_iter().map(|t|
        serde_json::json!({"file":t.file,"line":t.line,"target":t.target,"lines":t.trace_lines})).collect();
    let changes = compare(&baseline.fresh, &report.fresh)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)?;
    serde_json::to_writer_pretty(
        &mut output,
        &serde_json::json!({
            "baseline_sha256":format!("{:x}", Sha256::digest(&baseline_bytes)),
            "requested":requested,"traces":traces,"baseline_changes":changes,"report":report,
            "trace_limitations":["Legacy filters accept both zero/one-based lines and can include adjacent refs.",
                "Reference start lines may precede selector lines; missing traces are not evidence of extraction loss.",
                "Fresh/cold and pre-resolution trace records are not separately phase-tagged; use oracle reports for exact occurrence outcomes."]
        }),
    )?;
    println!(
        "{}",
        serde_json::json!({"requested":requested.len(),"traces":traces.len(),
        "baseline_changes":changes.len(),"fresh":report.fresh.counts,"snapshot_changes":report.snapshot_changes.len()})
    );
    Ok(())
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
