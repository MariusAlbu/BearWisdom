//! Translate persisted evidence into a pinned fixture identity domain.
//! No names, qualified names, confidence or strategies decide target equality.

use std::collections::{HashMap, HashSet};

use anyhow::{ensure, Context};

use super::QueryResult;
use crate::db::Database;
use crate::resolution_oracle::{
    DeclarationSite, FixtureFileId, Observation, ObservedBinding, ReferenceSite,
};
use crate::types::EdgeKind;

/// Read an explicitly selected file/kind cohort. Extraction input must come from
/// the same source snapshot as the DB, BEFORE resolver early exits. A missing log
/// row then means missing resolution, not missing extraction. Manifest file IDs
/// must also include targets outside the cohort. Legacy offsets and ambiguous
/// co-located sites are errors; neither is repaired by matching names.
pub fn observations(
    db: &Database,
    files: &HashMap<i64, FixtureFileId>,
    kinds: &[EdgeKind],
    extracted: &[ReferenceSite],
) -> QueryResult<Vec<Observation>> {
    Ok(read(db, files, kinds, extracted, false)?)
}

/// Identifier-anchored cohort. Unlike an expression start, a terminal selector
/// separates nested calls such as `a.begin().finish()`. Missing legacy anchors
/// are errors; neither names nor targets can supply an absent source identity.
pub fn selector_observations(
    db: &Database,
    files: &HashMap<i64, FixtureFileId>,
    kinds: &[EdgeKind],
    extracted: &[ReferenceSite],
) -> QueryResult<Vec<Observation>> {
    Ok(read(db, files, kinds, extracted, true)?)
}

fn read(
    db: &Database,
    files: &HashMap<i64, FixtureFileId>,
    kinds: &[EdgeKind],
    extracted: &[ReferenceSite],
    selectors: bool,
) -> anyhow::Result<Vec<Observation>> {
    let file_ids: HashSet<_> = files.values().copied().collect();
    ensure!(
        file_ids.len() == files.len(),
        "Fixture file IDs must be one-to-one with database files"
    );
    let mut slots = HashMap::new();
    let mut result = Vec::with_capacity(extracted.len());
    for site in extracted {
        ensure!(
            file_ids.contains(&site.file) && kinds.contains(&site.kind),
            "Extracted site outside oracle cohort: {site:?}"
        );
        ensure!(
            slots.insert(*site, result.len()).is_none(),
            "Ambiguous extracted site: {site:?}"
        );
        result.push(Observation {
            site: *site,
            binding: None,
        });
    }
    let declarations = declaration_multiplicity(db, files)?;
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT s.file_id, rr.source_byte, rr.kind, rr.outcome,
                t.file_id, t.line, t.col, t.kind, rr.source_selector_byte
         FROM ref_resolutions rr
         JOIN symbols s ON s.id = rr.source_id
         LEFT JOIN symbols t ON t.id = rr.target_id",
        )
        .context("Oracle requires source-byte evidence; reopen and reindex legacy databases")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let source_file: i64 = row.get(0)?;
        let Some(&file) = files.get(&source_file) else {
            continue;
        };
        // String decoding here is solely the persistence boundary.
        let kind: EdgeKind = row.get::<_, String>(2)?.parse()?;
        if !kinds.contains(&kind) {
            continue;
        }
        let byte_offset = row
            .get::<_, Option<u32>>(if selectors { 8 } else { 1 })?
            .context(
                "Legacy reference lacks source-byte evidence; reindex before oracle evaluation",
            )?;
        let site = ReferenceSite {
            file,
            byte_offset,
            kind,
        };
        let &slot = slots.get(&site).with_context(|| {
            format!("Resolution log site absent from extraction input: {site:?}")
        })?;
        ensure!(
            result[slot].binding.is_none(),
            "Ambiguous resolution log at {site:?}; cannot choose by name"
        );
        let binding = match row.get::<_, String>(3)?.as_str() {
            "resolved" => match row.get::<_, Option<i64>>(4)? {
                None => ObservedBinding::DanglingTarget,
                Some(target_file) => {
                    let target = DeclarationSite {
                        file: *files
                            .get(&target_file)
                            .context("Resolved target file missing from oracle manifest")?,
                        line: row.get(5)?,
                        col: row.get(6)?,
                        kind: row.get::<_, String>(7)?.parse()?,
                    };
                    ensure!(
                        declarations.get(&target) == Some(&1),
                        "Ambiguous declaration source address: {target:?}"
                    );
                    ObservedBinding::Resolved(target)
                }
            },
            "unresolved" => ObservedBinding::Unresolved,
            "drained" => ObservedBinding::Drained,
            other => anyhow::bail!("Unknown resolution outcome: {other}"),
        };
        result[slot].binding = Some(binding);
    }
    Ok(result)
}

fn declaration_multiplicity(
    db: &Database,
    files: &HashMap<i64, FixtureFileId>,
) -> anyhow::Result<HashMap<DeclarationSite, u64>> {
    let mut counts = HashMap::new();
    let mut stmt = db
        .conn()
        .prepare("SELECT file_id,line,col,kind FROM symbols")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let Some(&file) = files.get(&row.get::<_, i64>(0)?) else {
            continue;
        };
        let site = DeclarationSite {
            file,
            line: row.get(1)?,
            col: row.get(2)?,
            kind: row.get::<_, String>(3)?.parse()?,
        };
        *counts.entry(site).or_default() += 1;
    }
    Ok(counts)
}

#[cfg(test)]
#[path = "oracle_evidence_tests.rs"]
mod tests;
