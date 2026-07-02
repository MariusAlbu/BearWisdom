// =============================================================================
// query/unresolved_by_cause.rs — root-cause grouping for unresolved refs
//
// Groups `unresolved_refs` rows by the first-uncaptured-type cause the engine
// recorded at resolution-failure time (`cause_symbol_id` / `cause_kind`),
// ranked by distinct-ref count. Complements `unresolved_classify`'s surface-
// shape buckets: that answers "what does this ref's target look like", this
// answers "which upstream symbol's uncaptured type made it unresolvable".
// =============================================================================

use std::collections::HashMap;

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};

/// One sample unresolved ref inside a cause group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CauseSample {
    pub file: String,
    pub line: Option<u32>,
    pub target_name: String,
}

/// One root-cause group: the symbol whose own type was never captured, the
/// death-site kind that recorded it, and a sample of the refs that trace
/// back to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CauseGroup {
    pub cause_kind: String,
    pub cause_symbol_id: Option<i64>,
    pub cause_qualified_name: Option<String>,
    pub cause_file: Option<String>,
    pub cause_line: Option<u32>,
    pub ref_count: u64,
    pub samples: Vec<CauseSample>,
}

/// Full by-cause report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByCauseReport {
    /// Unresolved rows carrying a recorded cause.
    pub total_caused: u64,
    /// Unresolved rows with no recorded cause — the death site fell outside
    /// the instrumented ROOT/MEMBER paths.
    pub total_uncaused: u64,
    /// Groups sorted by `ref_count` desc, capped at `top_n`.
    pub groups: Vec<CauseGroup>,
}

/// Group every internal unresolved reference by its recorded cause.
///
/// `top_n` caps the number of groups returned (already sorted by size);
/// `samples_per_group` caps the number of sample refs kept per group.
pub fn unresolved_by_cause(
    db: &Database,
    top_n: usize,
    samples_per_group: usize,
) -> QueryResult<ByCauseReport> {
    let _timer = db.timer("unresolved_by_cause");
    let conn = db.conn();

    let scan_sql = format!(
        "SELECT u.cause_kind, u.cause_symbol_id, cs.qualified_name, cf.path, cs.line,
                u.target_name, u.source_line, f.path
         FROM unresolved_refs u
         JOIN symbols s        ON s.id = u.source_id
         JOIN files   f        ON f.id = s.file_id
         LEFT JOIN symbols cs  ON cs.id = u.cause_symbol_id
         LEFT JOIN files   cf  ON cf.id = cs.file_id
         WHERE f.origin = 'internal' AND {CODE_REF_FILTER} AND {GENERATED_FILE_FILTER}",
        CODE_REF_FILTER = crate::query::stats::CODE_REF_FILTER,
        GENERATED_FILE_FILTER = crate::query::stats::GENERATED_FILE_FILTER,
    );
    let mut stmt = conn
        .prepare(&scan_sql)
        .context("unresolved_by_cause: prepare scan")?;

    struct Agg {
        cause_kind: String,
        cause_symbol_id: Option<i64>,
        cause_qname: Option<String>,
        cause_file: Option<String>,
        cause_line: Option<u32>,
        count: u64,
        samples: Vec<CauseSample>,
    }

    // Groups key on (cause_kind, cause_symbol_id); a symbol-less cause
    // (unbound_root) has no id to group on, so the target name stands in as
    // the closest thing to an identity for that bucket.
    let mut groups: HashMap<(String, Option<i64>, String), Agg> = HashMap::new();
    let mut total_caused = 0u64;
    let mut total_uncaused = 0u64;

    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?, // cause_kind
                r.get::<_, Option<i64>>(1)?,    // cause_symbol_id
                r.get::<_, Option<String>>(2)?, // cause_qname
                r.get::<_, Option<String>>(3)?, // cause_file
                r.get::<_, Option<u32>>(4)?,    // cause_line
                r.get::<_, String>(5)?,         // target_name
                r.get::<_, Option<u32>>(6)?,    // source_line
                r.get::<_, String>(7)?,         // source file path
            ))
        })
        .context("unresolved_by_cause: execute scan")?;

    for row in rows {
        let Ok((
            cause_kind,
            cause_symbol_id,
            cause_qname,
            cause_file,
            cause_line,
            target_name,
            source_line,
            source_file,
        )) = row
        else {
            continue;
        };
        let Some(cause_kind) = cause_kind else {
            total_uncaused += 1;
            continue;
        };
        total_caused += 1;

        let name_fallback = if cause_symbol_id.is_none() {
            target_name.clone()
        } else {
            String::new()
        };
        let key = (cause_kind.clone(), cause_symbol_id, name_fallback);
        let agg = groups.entry(key).or_insert_with(|| Agg {
            cause_kind: cause_kind.clone(),
            cause_symbol_id,
            cause_qname: cause_qname.clone(),
            cause_file: cause_file.clone(),
            cause_line,
            count: 0,
            samples: Vec::new(),
        });
        agg.count += 1;
        if agg.samples.len() < samples_per_group {
            agg.samples.push(CauseSample {
                file: source_file,
                line: source_line,
                target_name,
            });
        }
    }

    let mut groups: Vec<CauseGroup> = groups
        .into_values()
        .map(|a| CauseGroup {
            cause_kind: a.cause_kind,
            cause_symbol_id: a.cause_symbol_id,
            cause_qualified_name: a.cause_qname,
            cause_file: a.cause_file,
            cause_line: a.cause_line,
            ref_count: a.count,
            samples: a.samples,
        })
        .collect();
    groups.sort_by(|a, b| {
        b.ref_count
            .cmp(&a.ref_count)
            .then_with(|| a.cause_kind.cmp(&b.cause_kind))
    });
    groups.truncate(top_n);

    Ok(ByCauseReport {
        total_caused,
        total_uncaused,
        groups,
    })
}

#[cfg(test)]
#[path = "unresolved_by_cause_tests.rs"]
mod tests;
