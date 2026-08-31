// =============================================================================
// indexer/include_assembly.rs — textual-include splice pre-pass
//
// A file pulled in by a textual include directive (an `Imports` ref carrying
// `ExtractedRef::is_include`) compiles as part of the INCLUDING unit's own
// scope: its top-level declarations belong to the unit's namespace, not to a
// module of their own. The extractor indexes such a fragment as a standalone
// file with bare qualified names, severing the declaring-container identity.
//
// This pass restores that identity before the Compilation is built: each
// include ref's stem is resolved to an indexed file (same-directory probe
// first, then sibling directories of the including file), and the resolved
// fragment's symbols are re-parented under the including unit's namespace —
// in the ParsedFile batch, in the (path, qname) → id map, and in the
// persisted symbol rows. The Compilation's qname-truncation member fallback
// and the cross-file containment binder then see the fragment's symbols as
// members of the unit.
//
// The pass is data-driven: it activates only on `is_include` refs and joins
// stems purely on path strings, so project fragments and supplied
// virtual-path fragments (`ext:<eco>:<pkg>/dir/file.inc`) resolve through
// the same directory probe.
// =============================================================================

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{Context, Result};

use crate::db::Database;
use crate::indexer::write::SymbolIdMap;
use crate::types::{ParsedFile, SymbolKind};

/// One planned splice: the fragment at `included_idx` joins the namespace
/// `ns_qname` declared by the including unit.
struct Splice {
    included_idx: usize,
    ns_qname: String,
}

/// A persisted-row rewrite produced by `apply_splices`: the symbol keeps its
/// id; its qualified name, scope path, and the qname-bearing prefix of its
/// `symbol_key` change.
struct RowUpdate {
    id: i64,
    new_qname: String,
    new_scope_path: Option<String>,
    old_key_prefix: String,
    new_key_prefix: String,
}

/// Resolve every include ref to its fragment file, re-parent the fragments'
/// symbols under the including unit's namespace, and persist the rewritten
/// symbol rows. Returns the number of re-parented symbols. Idempotent: a
/// spliced fragment's top-level symbols carry a `scope_path`, which excludes
/// the fragment from further claims.
pub fn assemble_includes(
    db: &Database,
    parsed: &mut [ParsedFile],
    symbol_id_map: &mut SymbolIdMap,
) -> Result<usize> {
    let _t = crate::indexer::phase_timer::scope("include_assembly");
    let splices = plan_splices(parsed);
    if splices.is_empty() {
        return Ok(0);
    }
    let (reparented, updates) = apply_splices(parsed, &splices, symbol_id_map);
    persist_updates(db, &updates)?;
    if reparented > 0 {
        tracing::info!(
            "Include assembly re-parented {reparented} symbols into including units"
        );
    }
    Ok(reparented)
}

// ---------------------------------------------------------------------------
// Planning — resolve include stems to fragment files
// ---------------------------------------------------------------------------

/// Resolve include stems to fragment files, breadth-first through nested
/// includes (a claimed fragment's own include refs splice further fragments
/// into the same namespace, matching textual-inclusion semantics). Claim
/// order is deterministic — units sorted by path, the lexicographically
/// smallest candidate wins each probe — and a fragment is claimed at most
/// once.
fn plan_splices(parsed: &[ParsedFile]) -> Vec<Splice> {
    let mut by_stem: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, pf) in parsed.iter().enumerate() {
        by_stem.entry(stem_lower(&pf.path)).or_default().push(i);
    }
    for list in by_stem.values_mut() {
        list.sort_by(|&a, &b| parsed[a].path.cmp(&parsed[b].path));
    }

    let mut unit_indices: Vec<usize> = parsed
        .iter()
        .enumerate()
        .filter(|(_, pf)| {
            pf.refs.iter().any(|r| r.is_include) && namespace_qname(pf).is_some()
        })
        .map(|(i, _)| i)
        .collect();
    unit_indices.sort_by(|&a, &b| parsed[a].path.cmp(&parsed[b].path));

    let mut claimed: HashSet<usize> = HashSet::new();
    let mut splices = Vec::new();
    for unit_idx in unit_indices {
        let Some(ns_qname) = namespace_qname(&parsed[unit_idx]).map(str::to_string) else {
            continue;
        };
        let mut queue: VecDeque<usize> = VecDeque::from([unit_idx]);
        while let Some(cur) = queue.pop_front() {
            for r in parsed[cur].refs.iter().filter(|r| r.is_include) {
                let stem = r.module.as_deref().unwrap_or(&r.target_name);
                let Some(frag) = resolve_stem(parsed, &by_stem, cur, stem, &claimed) else {
                    continue;
                };
                claimed.insert(frag);
                splices.push(Splice {
                    included_idx: frag,
                    ns_qname: ns_qname.clone(),
                });
                queue.push_back(frag);
            }
        }
    }
    splices
}

/// Directory probe for one include stem, relative to the file at `from_idx`:
/// a same-directory fragment wins; otherwise the first fragment in a sibling
/// directory. Candidates must share the including file's language, be
/// unclaimed fragments, and not be the including file itself.
fn resolve_stem(
    parsed: &[ParsedFile],
    by_stem: &HashMap<String, Vec<usize>>,
    from_idx: usize,
    stem: &str,
    claimed: &HashSet<usize>,
) -> Option<usize> {
    let candidates = by_stem.get(&stem.to_ascii_lowercase())?;
    let from_dir = dir_lower(&parsed[from_idx].path);
    let from_parent = parent_dir(&from_dir);
    let lang = parsed[from_idx].language.as_str();
    let eligible = |i: usize| {
        i != from_idx
            && !claimed.contains(&i)
            && parsed[i].language == lang
            && is_fragment(&parsed[i])
    };
    candidates
        .iter()
        .copied()
        .find(|&i| eligible(i) && dir_lower(&parsed[i].path) == from_dir)
        .or_else(|| {
            candidates
                .iter()
                .copied()
                .find(|&i| eligible(i) && parent_dir(&dir_lower(&parsed[i].path)) == from_parent)
        })
}

/// A splice-eligible fragment: has at least one top-level symbol, none of its
/// top-level symbols declares a namespace of its own, and none was already
/// spliced (a spliced top-level symbol carries the unit namespace in
/// `scope_path`).
fn is_fragment(pf: &ParsedFile) -> bool {
    let mut has_top_level = false;
    for s in &pf.symbols {
        if s.parent_index.is_some() {
            continue;
        }
        has_top_level = true;
        if s.kind == SymbolKind::Namespace || s.scope_path.is_some() {
            return false;
        }
    }
    has_top_level
}

/// The unit/program namespace a file declares: its first top-level Namespace
/// symbol's qualified name.
fn namespace_qname(pf: &ParsedFile) -> Option<&str> {
    pf.symbols
        .iter()
        .find(|s| s.parent_index.is_none() && s.kind == SymbolKind::Namespace)
        .map(|s| s.qualified_name.as_str())
}

// ---------------------------------------------------------------------------
// Application — re-parent fragment symbols under the unit namespace
// ---------------------------------------------------------------------------

/// Prefix every symbol qname (and alias-target key) in each spliced fragment
/// with the unit's namespace qname, set top-level scope paths to the
/// namespace, and rekey the (path, qname) → id map so the Compilation build
/// still finds each symbol's id. Returns the re-parented symbol count and
/// the row rewrites to persist.
fn apply_splices(
    parsed: &mut [ParsedFile],
    splices: &[Splice],
    symbol_id_map: &mut SymbolIdMap,
) -> (usize, Vec<RowUpdate>) {
    let mut updates = Vec::new();
    let mut reparented = 0usize;
    for splice in splices {
        let prefix = splice.ns_qname.as_str();
        let pf = &mut parsed[splice.included_idx];
        for sym in &mut pf.symbols {
            let old_qname = std::mem::take(&mut sym.qualified_name);
            let new_qname = format!("{prefix}.{old_qname}");
            sym.qualified_name = new_qname.clone();
            sym.scope_path = match sym.scope_path.take() {
                Some(sp) => Some(format!("{prefix}.{sp}")),
                None if sym.parent_index.is_none() => Some(prefix.to_string()),
                None => None,
            };
            reparented += 1;
            if let Some(id) = symbol_id_map.remove(&(pf.path.clone(), old_qname.clone())) {
                symbol_id_map.insert((pf.path.clone(), new_qname.clone()), id);
                updates.push(RowUpdate {
                    id,
                    new_qname: new_qname.clone(),
                    new_scope_path: sym.scope_path.clone(),
                    old_key_prefix: format!("{old_qname}#"),
                    new_key_prefix: format!("{new_qname}#"),
                });
            }
        }
        for (qname, _) in &mut pf.alias_targets {
            *qname = format!("{prefix}.{qname}");
        }
    }
    (reparented, updates)
}

// ---------------------------------------------------------------------------
// Persistence — rewrite the already-written symbol rows
// ---------------------------------------------------------------------------

/// Rewrite each re-parented symbol's row in place: new qualified name and
/// scope path, and the qname prefix inside `symbol_key` (the `#` anchors the
/// end of the qname portion, so only that portion is substituted).
fn persist_updates(db: &Database, updates: &[RowUpdate]) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin include-assembly transaction")?;
    {
        let mut stmt = tx
            .prepare(
                "UPDATE symbols SET qualified_name = ?1, scope_path = ?2, \
                        symbol_key = REPLACE(symbol_key, ?3, ?4) \
                 WHERE id = ?5",
            )
            .context("Failed to prepare include-assembly update")?;
        for u in updates {
            stmt.execute(rusqlite::params![
                u.new_qname,
                u.new_scope_path,
                u.old_key_prefix,
                u.new_key_prefix,
                u.id
            ])
            .context("Failed to rewrite re-parented symbol row")?;
        }
    }
    tx.commit()
        .context("Failed to commit include-assembly transaction")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Path helpers — plain string splits over '/'-separated index paths
// ---------------------------------------------------------------------------

/// Case-folded file stem: basename with the final extension stripped.
fn stem_lower(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    stem.to_ascii_lowercase()
}

/// Case-folded directory prefix (empty for a bare filename). Virtual paths
/// (`ext:<eco>:<pkg>/dir/file`) split the same way — the scheme prefix stays
/// part of the directory string, so joins stay within one virtual root.
fn dir_lower(path: &str) -> String {
    match path.rfind(['/', '\\']) {
        Some(pos) => path[..pos].to_ascii_lowercase(),
        None => String::new(),
    }
}

/// Parent of a directory string (empty at a root).
fn parent_dir(dir: &str) -> String {
    match dir.rfind(['/', '\\']) {
        Some(pos) => dir[..pos].to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "include_assembly_tests.rs"]
mod tests;
