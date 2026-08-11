// =============================================================================
// query/drain_audit — flag drained refs that mask indexable declarations
//
// A drain (`builtin_skip`) asserts a name is language-defined vocabulary with
// no declaration to bind — a stronger claim than "unresolved", because the
// drain rung runs before every project-symbol lookup. This audit re-checks
// that claim against the index: for every distinct drained
// (language, edge kind, target name), it probes for declarations the resolve
// ladder's own kind gate would accept. A hit means the drain is masking a
// bindable symbol — either a collision the drain list missed, or supply that
// arrived after the drain was written — and the name should come off the
// drain list, not stay written off.
// =============================================================================

use std::collections::BTreeMap;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::languages;
use crate::query::error::QueryResult;
use crate::type_checker::profile::chain_specs::{
    KindCompatibility, KindTable, PERMISSIVE_KIND_TABLE,
};
use crate::types::{EdgeKind, SymbolKind};

/// Cap on declaration matches reported per finding; `total_matches` carries
/// the uncapped count.
const MATCHES_PER_FINDING: usize = 5;

/// A declaration that a drained name would bind to if it were not drained.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrainAuditMatch {
    pub qualified_name: String,
    pub symbol_kind: String,
    /// `internal` or `external` — internal means a project symbol is being
    /// masked (a collision-guard miss); external means dependency supply
    /// exists for the name (drain rot).
    pub origin: String,
    pub file_path: String,
    /// False when the match required case folding. Only meaningful for
    /// case-insensitive languages; a case-folded hit in a case-sensitive
    /// language is informational, not a bind the ladder would make.
    pub case_exact: bool,
}

/// One drained (language, edge kind, target name) group with at least one
/// kind-compatible declaration in the index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrainAuditFinding {
    pub language: String,
    pub kind: String,
    pub target_name: String,
    /// How many refs are currently drained under this group.
    pub drained_count: u32,
    /// Kind-compatible declarations found, capped at `MATCHES_PER_FINDING`.
    pub matches: Vec<DrainAuditMatch>,
    pub total_matches: u32,
}

/// Audit every drained ref group against the symbol index. Findings are
/// ordered by `drained_count` descending — the biggest masked groups first.
pub fn drain_audit(db: &Database) -> QueryResult<Vec<DrainAuditFinding>> {
    let _timer = db.timer("drain_audit");
    let conn = db.conn();

    let mut groups: Vec<(String, String, String, u32)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT f.language, u.kind, u.target_name, COUNT(*)
             FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE u.drained = 1 AND f.origin = 'internal'
             GROUP BY f.language, u.kind, u.target_name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        for row in rows {
            groups.push(row?);
        }
    }
    if groups.is_empty() {
        return Ok(Vec::new());
    }

    let registry = languages::default_registry();
    let mut tables: BTreeMap<String, KindTable> = BTreeMap::new();

    let mut findings = Vec::new();
    // A drain can only mask what the ladder could bind: a declaration in the
    // SAME language as the drained ref. Cross-language same-named symbols
    // (java.lang.Integer vs a pascal Integer cast) are unreachable from that
    // ladder and would be pure audit noise.
    let mut stmt = conn.prepare(
        "SELECT s.name, s.qualified_name, s.kind, f.origin, f.path
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 COLLATE NOCASE
           AND COALESCE(s.origin_language, f.language) = ?2
           AND NOT (s.kind = 'variable' AND s.qualified_name <> s.name)",
    )?;

    for (language, kind_str, target, drained_count) in groups {
        let table = *tables.entry(language.clone()).or_insert_with(|| {
            registry
                .get_dedicated(&language)
                .and_then(|p| p.profile())
                .map_or(PERMISSIVE_KIND_TABLE, |p| p.kind_compatible_table)
        });
        let edge_kind = EdgeKind::from_str(&kind_str).ok();

        let mut matches = Vec::new();
        let mut total: u32 = 0;
        let rows = stmt.query_map([&target, &language], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (name, qname, sym_kind, origin, path) = row?;
            // An unparseable symbol kind stays visible (permissive), matching
            // the ladder's own kind gate; a parseable one must pass the
            // language's table for the drained edge kind.
            let compatible = match (edge_kind, SymbolKind::from_str(&sym_kind)) {
                (Some(ek), Ok(sk)) => KindCompatibility::check(table, ek, sk),
                _ => true,
            };
            if !compatible {
                continue;
            }
            total += 1;
            if matches.len() < MATCHES_PER_FINDING {
                matches.push(DrainAuditMatch {
                    qualified_name: qname,
                    symbol_kind: sym_kind,
                    origin,
                    file_path: path,
                    case_exact: name == target,
                });
            }
        }
        if total > 0 {
            findings.push(DrainAuditFinding {
                language,
                kind: kind_str,
                target_name: target,
                drained_count,
                matches,
                total_matches: total,
            });
        }
    }

    findings.sort_by(|a, b| b.drained_count.cmp(&a.drained_count));
    Ok(findings)
}

#[cfg(test)]
#[path = "drain_audit_tests.rs"]
mod tests;
