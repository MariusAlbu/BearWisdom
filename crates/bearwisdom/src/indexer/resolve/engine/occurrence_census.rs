//! Per-file accounting at the resolver boundary. Counters never choose a target.

use anyhow::{Context, Result};
use rustc_hash::FxHashMap;

use crate::occurrence::{Disposition, OccurrenceBucket};
use crate::types::{EdgeKind, ParsedFile};

/// One file's census, including files with zero symbols or zero references.
/// Borrow language labels while counting to avoid a String allocation per ref.
pub(super) struct FileCensus<'a> {
    file: &'a ParsedFile,
    counts: FxHashMap<(&'a str, EdgeKind, bool, Disposition), u64>,
}

impl<'a> FileCensus<'a> {
    pub(super) fn new(file: &'a ParsedFile) -> Self {
        Self {
            file,
            counts: FxHashMap::default(),
        }
    }

    pub(super) fn record(&mut self, ref_index: usize, disposition: Disposition) {
        let r = &self.file.refs[ref_index];
        let language = self
            .file
            .ref_origin_languages
            .get(ref_index)
            .and_then(|l| l.as_deref())
            .unwrap_or(&self.file.language);
        let snippet = self
            .file
            .symbol_from_snippet
            .get(r.source_symbol_index)
            .copied()
            .unwrap_or(false);
        *self
            .counts
            .entry((language, r.kind, snippet, disposition))
            .or_default() += 1;
    }

    pub(super) fn buckets(&self) -> Vec<OccurrenceBucket> {
        let mut buckets: Vec<_> = self
            .counts
            .iter()
            .map(
                |(&(language, kind, from_snippet, disposition), &count)| OccurrenceBucket {
                    language: language.to_owned(),
                    kind,
                    from_snippet,
                    disposition,
                    count,
                },
            )
            .collect();
        // Persistence/display boundary; deterministic ordering is not binding identity.
        buckets.sort_by_key(|b| {
            (
                b.language.clone(),
                b.kind as u8,
                b.from_snippet,
                b.disposition as u8,
            )
        });
        buckets
    }
}

/// Called INSIDE the edge/log transaction. Incremental passes replace only
/// measured files; FK cascade removes deleted files. The file hash detects a
/// parse write whose resolution pass has not completed (or failed).
pub(super) fn persist(
    tx: &rusqlite::Transaction<'_>,
    censuses: &[FileCensus<'_>],
    clear_existing: bool,
) -> Result<()> {
    if clear_existing {
        tx.execute("DELETE FROM resolution_census", [])?;
    }
    let mut insert = tx.prepare_cached(
        "INSERT INTO resolution_census (file_id, content_hash, counts_json)
         SELECT id, ?2, ?3 FROM files WHERE path = ?1
         ON CONFLICT(file_id) DO UPDATE SET
             content_hash = excluded.content_hash, counts_json = excluded.counts_json",
    )?;
    for census in censuses {
        let buckets = census.buckets();
        anyhow::ensure!(
            buckets.iter().map(|b| b.count).sum::<u64>() == census.file.refs.len() as u64,
            "Incomplete occurrence census for {}",
            census.file.path,
        );
        let counts = serde_json::to_string(&buckets)?;
        let written = insert
            .execute(rusqlite::params![
                census.file.path,
                census.file.content_hash,
                counts
            ])
            .context("Failed to persist occurrence census for an indexed file")?;
        anyhow::ensure!(
            written == 1,
            "Census file is not indexed: {}",
            census.file.path
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "occurrence_census_tests.rs"]
mod tests;
