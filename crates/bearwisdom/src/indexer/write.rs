// =============================================================================
// indexer/write.rs  —  shared write pipeline
//
// Single source of truth for writing parsed files to the database.
// Both full and incremental indexers call these functions — no more
// duplicated SQL or diverging statement preparation strategies.
// =============================================================================

use crate::db::Database;
use crate::symbol_key::{is_mergeable, symbol_key};
use crate::type_checker::core::types::TypeArena;
use crate::types::ParsedFile;
use anyhow::{Context, Result};
use rusqlite::types::Value;
use rusqlite::OptionalExtension;
use std::collections::{HashMap, HashSet};
use tracing::{debug, warn};

// Batching constants. SQLite's `SQLITE_MAX_VARIABLE_NUMBER` defaults to
// 32766 on modern builds; 128 rows × 14 vars = 1792 variables, well
// inside any realistic limit. Row counts are chosen so almost every file
// fits in one batch (median C# file has < 128 symbols) — a bigger batch
// would only save us on outlier files and risk hitting the variable
// cap on pathological generated code.
const SYMBOL_COLS: usize = 16;
const SYMBOL_BATCH_ROWS: usize = 128;
const IMPORT_COLS: usize = 5;
const IMPORT_BATCH_ROWS: usize = 256;

/// Maps relative_path → SQLite file row ID.
pub type FileIdMap = HashMap<String, i64>;

/// Maps (relative_path, qualified_name) → SQLite symbol row ID.
pub type SymbolIdMap = HashMap<(String, String), i64>;

fn symbol_insert_sql(rows: usize) -> String {
    // Pre-sized: 14 vars per row, one tuple plus a separator of 3 chars,
    // plus the header/footer. 64 is a comfortable overshoot.
    let mut sql = String::with_capacity(256 + rows * 64);
    sql.push_str(
        "INSERT INTO symbols \
         (file_id, name, qualified_name, kind, line, col, end_line, end_col, \
          scope_path, signature, doc_comment, visibility, origin, origin_language, \
          symbol_key, mergeable) \
         VALUES ",
    );
    for i in 0..rows {
        if i > 0 { sql.push(','); }
        sql.push_str("(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)");
    }
    sql.push_str(" RETURNING id");
    sql
}

fn import_insert_sql(rows: usize) -> String {
    let mut sql = String::with_capacity(128 + rows * 24);
    sql.push_str(
        "INSERT INTO imports (file_id, imported_name, module_path, alias, line) VALUES ",
    );
    for i in 0..rows {
        if i > 0 { sql.push(','); }
        sql.push_str("(?,?,?,?,?)");
    }
    sql
}

fn push_symbol_params(
    params: &mut Vec<Value>,
    file_id: i64,
    pf: &ParsedFile,
    global_idx: usize,
    origin: &str,
    arena: Option<&TypeArena>,
) {
    let sym = &pf.symbols[global_idx];
    let origin_language: Option<&str> = pf
        .symbol_origin_languages
        .get(global_idx)
        .and_then(|o| o.as_deref());
    // The effective language drives the mergeable/key dialect: a spliced
    // sub-symbol (TS in a .vue) keys under its own language, not the host's.
    let language = origin_language.unwrap_or(pf.language.as_str());
    params.push(Value::Integer(file_id));
    params.push(Value::Text(sym.name.clone()));
    params.push(Value::Text(sym.qualified_name.clone()));
    params.push(Value::Text(sym.kind.as_str().to_string()));
    params.push(Value::Integer(sym.start_line as i64));
    params.push(Value::Integer(sym.start_col as i64));
    params.push(Value::Integer(sym.end_line as i64));
    params.push(Value::Integer(sym.end_col as i64));
    params.push(match &sym.scope_path {
        Some(s) => Value::Text(s.clone()),
        None => Value::Null,
    });
    params.push(match &sym.signature {
        Some(s) => Value::Text(s.clone()),
        None => Value::Null,
    });
    params.push(match &sym.doc_comment {
        Some(s) => Value::Text(s.clone()),
        None => Value::Null,
    });
    params.push(match sym.visibility {
        Some(v) => Value::Text(v.as_str().to_string()),
        None => Value::Null,
    });
    params.push(Value::Text(origin.to_string()));
    params.push(match origin_language {
        Some(s) => Value::Text(s.to_string()),
        None => Value::Null,
    });
    // Stable identity (SYMBOL-IDENTITY.md): contract-derived key + the merge
    // flag that decides whether multiple files collapse to one logical symbol.
    // The key needs the arena that interned this symbol's param TypeIds — the
    // legacy no-arena parse path can't format them, so the key stays NULL there
    // (those rows fall back to the churn identity until re-indexed with an arena).
    params.push(match arena {
        Some(a) => Value::Text(symbol_key(language, sym, file_id, a)),
        None => Value::Null,
    });
    params.push(Value::Integer(
        is_mergeable(language, sym.kind, sym.signature.as_deref()) as i64,
    ));
}

/// Batched symbol insert. Replaces the per-row loop: for a file with 400
/// symbols, the old path executed 400 separate `INSERT … RETURNING id`
/// statements; this path runs 4 chunked `INSERT … VALUES (…),(…),…
/// RETURNING id` statements, cutting the rusqlite round-trip count by
/// ~100×. Preserves the SymbolIdMap ordering the rest of the pipeline
/// assumes: SQLite's RETURNING returns rows in VALUES order.
fn insert_symbols_batched(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
    origin: &str,
    symbol_id_map: &mut SymbolIdMap,
    arena: Option<&TypeArena>,
) -> Result<()> {
    if pf.symbols.is_empty() { return Ok(()); }

    let total = pf.symbols.len();
    // Positional id capture (RETURNING is in VALUES order) so containment and
    // location writes key on the exact row, not a qname that overloads share.
    let mut id_by_idx: Vec<i64> = vec![0; total];
    let mut start = 0;
    while start < total {
        let end = (start + SYMBOL_BATCH_ROWS).min(total);
        let rows = end - start;
        let sql = symbol_insert_sql(rows);
        let mut params: Vec<Value> = Vec::with_capacity(rows * SYMBOL_COLS);
        for i in start..end {
            push_symbol_params(&mut params, file_id, pf, i, origin, arena);
        }

        // prepare_cached hits when the chunk is exactly SYMBOL_BATCH_ROWS
        // (true for every non-tail chunk of a large file). The tail chunk
        // is a one-shot prepare, negligible.
        let mut stmt = tx
            .prepare_cached(&sql)
            .context("Failed to prepare batched symbol insert")?;
        let ids: Vec<i64> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))
            .context("Failed to execute batched symbol insert")?
            .collect::<std::result::Result<_, _>>()
            .context("Failed to collect RETURNING ids")?;

        if ids.len() != rows {
            anyhow::bail!(
                "RETURNING id count mismatch: expected {} rows, got {} for {}",
                rows, ids.len(), pf.path,
            );
        }
        for (i, sym_id) in (start..end).zip(ids.iter()) {
            let sym = &pf.symbols[i];
            symbol_id_map.insert((pf.path.clone(), sym.qualified_name.clone()), *sym_id);
            id_by_idx[i] = *sym_id;
        }
        start = end;
    }
    write_containment_and_locations(tx, file_id, pf, &id_by_idx)?;
    Ok(())
}

/// Write the structural containment edge (`containing_id`) and a declaration
/// location per symbol. `id_by_idx[k]` is the row id of `pf.symbols[k]`.
///
/// Containment is intra-file here: a symbol's `parent_index` points within the
/// same file's symbol vec, so the parent's id is already known. A member whose
/// parent lives in another file (a Rust `impl` method, a C# partial member
/// declared apart from its class) keeps `containing_id` NULL until the
/// survivor-matching pass can resolve it through the global key→id map.
fn write_containment_and_locations(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
    id_by_idx: &[i64],
) -> Result<()> {
    // One declaration location per symbol (mergeable multi-location merging is
    // a survivor-matching concern; on the churn path each file owns its row).
    {
        const LOC_COLS: usize = 4;
        const LOC_BATCH_ROWS: usize = 256;
        let total = id_by_idx.len();
        let mut start = 0;
        while start < total {
            let end = (start + LOC_BATCH_ROWS).min(total);
            let rows = end - start;
            let mut sql = String::with_capacity(96 + rows * 12);
            sql.push_str("INSERT OR IGNORE INTO symbol_locations (symbol_id, file_id, line, col) VALUES ");
            for i in 0..rows {
                if i > 0 { sql.push(','); }
                sql.push_str("(?,?,?,?)");
            }
            let mut params: Vec<Value> = Vec::with_capacity(rows * LOC_COLS);
            for i in start..end {
                let sym = &pf.symbols[i];
                params.push(Value::Integer(id_by_idx[i]));
                params.push(Value::Integer(file_id));
                params.push(Value::Integer(sym.start_line as i64));
                params.push(Value::Integer(sym.start_col as i64));
            }
            tx.prepare_cached(&sql)
                .context("Failed to prepare symbol_locations insert")?
                .execute(rusqlite::params_from_iter(params.iter()))
                .context("Failed to insert symbol_locations")?;
            start = end;
        }
    }

    // Containment edge for the symbols that have an in-file parent.
    let mut upd = tx
        .prepare_cached("UPDATE symbols SET containing_id = ?1 WHERE id = ?2")
        .context("Failed to prepare containing_id update")?;
    for (i, sym) in pf.symbols.iter().enumerate() {
        if let Some(p) = sym.parent_index {
            if p < id_by_idx.len() {
                upd.execute(rusqlite::params![id_by_idx[p], id_by_idx[i]])?;
            }
        }
    }
    Ok(())
}

/// Batched import insert. Filters non-Imports refs, then chunks.
fn insert_imports_batched(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
) -> Result<()> {
    let imports: Vec<&crate::types::ExtractedRef> = pf
        .refs
        .iter()
        .filter(|r| r.kind == crate::types::EdgeKind::Imports)
        .collect();
    if imports.is_empty() { return Ok(()); }

    let mut start = 0;
    while start < imports.len() {
        let end = (start + IMPORT_BATCH_ROWS).min(imports.len());
        let rows = end - start;
        let sql = import_insert_sql(rows);
        let mut params: Vec<Value> = Vec::with_capacity(rows * IMPORT_COLS);
        for r in &imports[start..end] {
            params.push(Value::Integer(file_id));
            params.push(Value::Text(r.target_name.clone()));
            params.push(match r.module.as_deref() {
                Some(s) => Value::Text(s.to_string()),
                None => Value::Null,
            });
            params.push(Value::Null); // alias — always null in extract
            params.push(Value::Integer(r.line as i64));
        }

        tx.prepare_cached(&sql)
            .context("Failed to prepare batched import insert")?
            .execute(rusqlite::params_from_iter(params.iter()))
            .context("Failed to execute batched import insert")?;
        start = end;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Core write: files + symbols + imports + routes
// ---------------------------------------------------------------------------

/// What a survivor-matching write learned about the blast radius. Both sets
/// are derived from the key diff, not from "every changed file": only a symbol
/// whose key VANISHED forces its consumers to re-resolve, and only a
/// genuinely-new key can satisfy a previously-unresolved reference. A symbol
/// whose key SURVIVES keeps its id, so its consumers' edges remain valid and it
/// triggers nothing (SYMBOL-IDENTITY.md §4).
#[derive(Default, Debug)]
pub struct SurvivorReport {
    /// Paths of files holding an edge INTO a symbol that vanished in this write.
    /// Excludes the rewritten files themselves (they re-resolve regardless).
    pub vanished_dependent_paths: HashSet<String>,
    /// Names of symbols inserted under a brand-new key. A file with an
    /// unresolved ref to one of these names may now bind.
    pub new_symbol_names: HashSet<String>,
}

/// Incremental write with stable identity (SYMBOL-IDENTITY.md §4).
///
/// Instead of deleting every symbol in a changed file and reinserting with
/// fresh ids (which cascade-drops all inbound edges and forces every dependent
/// to re-resolve), this matches each new symbol to its existing row by
/// `symbol_key`. Survivors keep their id; only vanished keys are deleted and
/// only new keys are inserted. The returned `SurvivorReport` carries the
/// narrowed blast radius.
///
/// Returns:
///   - `FileIdMap`: relative path → file row ID
///   - `SymbolIdMap`: (relative_path, qualified_name) → symbol row ID (survivor
///      ids included, so the resolver attributes re-resolved refs to the
///      stable id)
///   - `SurvivorReport`: vanished-dependent paths + new-symbol names
pub fn write_parsed_files_incremental(
    db: &Database,
    parsed: &[ParsedFile],
    arena: Option<&TypeArena>,
) -> Result<(FileIdMap, SymbolIdMap, SurvivorReport)> {
    let conn = db.conn();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin incremental write transaction")?;

    let mut file_id_map: FileIdMap = HashMap::new();
    let mut symbol_id_map: SymbolIdMap = HashMap::new();
    let mut report = SurvivorReport::default();

    for pf in parsed {
        let file_id = upsert_file_row(&tx, pf, now, "internal")?;
        file_id_map.insert(pf.path.clone(), file_id);

        // Imports carry no stable identity — rebuilding is cheaper than diffing.
        tx.prepare_cached("DELETE FROM imports WHERE file_id = ?1")
            .context("Failed to prepare import delete")?
            .execute([file_id])?;

        survivor_match_file(&tx, file_id, pf, "internal", arena, &mut symbol_id_map, &mut report)?;

        insert_routes_incremental(&tx, file_id, pf, &symbol_id_map)?;
        insert_imports_batched(&tx, file_id, pf)?;
    }

    tx.commit()
        .context("Failed to commit incremental write transaction")?;

    if let Some(ref cache) = db.query_cache {
        cache.invalidate_all();
    }

    Ok((file_id_map, symbol_id_map, report))
}

/// Upsert the `files` row and return its id. Shared by the survivor-matching
/// write; the full/external path keeps its own inline upsert.
fn upsert_file_row(
    tx: &rusqlite::Transaction<'_>,
    pf: &ParsedFile,
    now: i64,
    origin: &str,
) -> Result<i64> {
    tx.prepare_cached(
        "INSERT INTO files (path, hash, language, last_indexed, mtime, size, package_id, origin)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(path) DO UPDATE SET
           hash = excluded.hash,
           language = excluded.language,
           last_indexed = excluded.last_indexed,
           mtime = excluded.mtime,
           size = excluded.size,
           package_id = excluded.package_id,
           origin = excluded.origin
         RETURNING id",
    )
    .context("Failed to prepare file upsert")?
    .query_row(
        rusqlite::params![pf.path, pf.content_hash, pf.language, now, pf.mtime, pf.size as i64, pf.package_id, origin],
        |r| r.get(0),
    )
    .with_context(|| format!("Failed to upsert file {}", pf.path))
}

/// Reconcile file F's symbols against the persisted graph by `symbol_key`.
/// Populates `symbol_id_map` with the resolved id of every current symbol
/// (survivor or freshly inserted) and records vanished-dependents / new-symbol
/// names into `report`.
fn survivor_match_file(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
    origin: &str,
    arena: Option<&TypeArena>,
    symbol_id_map: &mut SymbolIdMap,
    report: &mut SurvivorReport,
) -> Result<()> {
    let total = pf.symbols.len();
    let mut id_by_idx: Vec<i64> = vec![0; total];

    // --- Load the deletion universe + non-mergeable survivor candidates ---
    // Every primary-in-F row is a candidate. Non-mergeable rows are matched by
    // their (file-scoped) key; rows with a NULL key (legacy / no-arena writes)
    // can never match a keyed new symbol, so they fall into "vanished" and are
    // re-inserted keyed — a one-time migration.
    let mut local_by_key: HashMap<String, i64> = HashMap::new();
    let mut all_local_nonmerge: Vec<i64> = Vec::new();
    {
        let mut stmt = tx
            .prepare_cached("SELECT id, symbol_key, mergeable FROM symbols WHERE file_id = ?1")
            .context("Failed to prepare existing-symbol load")?;
        let rows = stmt.query_map([file_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (id, key, mergeable) = row?;
            if mergeable == 0 {
                all_local_nonmerge.push(id);
                if let Some(k) = key {
                    local_by_key.insert(k, id);
                }
            }
        }
    }

    // Mergeable symbols with a declaration site in F (primary here or elsewhere).
    let mut merge_located_ids: HashSet<i64> = HashSet::new();
    {
        let mut stmt = tx
            .prepare_cached(
                "SELECT DISTINCT s.id FROM symbols s
                 JOIN symbol_locations l ON l.symbol_id = s.id
                 WHERE l.file_id = ?1 AND s.mergeable = 1",
            )
            .context("Failed to prepare mergeable-location load")?;
        let rows = stmt.query_map([file_id], |r| r.get::<_, i64>(0))?;
        for row in rows {
            merge_located_ids.insert(row?);
        }
    }

    let mut matched_nonmerge: HashSet<i64> = HashSet::new();
    let mut mergeable_still_in_f: HashSet<i64> = HashSet::new();
    let mut used_ids: HashSet<i64> = HashSet::new();

    // --- Reconcile each current symbol ---
    for (idx, sym) in pf.symbols.iter().enumerate() {
        let language = pf
            .symbol_origin_languages
            .get(idx)
            .and_then(|o| o.as_deref())
            .unwrap_or(pf.language.as_str());
        let mergeable = is_mergeable(language, sym.kind, sym.signature.as_deref());
        let key = arena.map(|a| symbol_key(language, sym, file_id, a));

        // A survivor is the existing row carrying this exact key: file-scoped for
        // non-mergeable symbols, global for mergeable ones.
        let survivor: Option<i64> = match (&key, mergeable) {
            (Some(k), true) => tx
                .prepare_cached(
                    "SELECT id FROM symbols WHERE symbol_key = ?1 AND mergeable = 1 LIMIT 1",
                )?
                .query_row([k], |r| r.get::<_, i64>(0))
                .optional()?,
            (Some(k), false) => local_by_key.get(k).copied(),
            (None, _) => None,
        };

        let id = match survivor {
            // `used_ids` guards against two new symbols colliding on one key
            // (extractor ambiguity, SYMBOL-IDENTITY.md §8): the first claims the
            // survivor, the rest are inserted fresh so no id is double-assigned.
            Some(existing) if !used_ids.contains(&existing) => {
                used_ids.insert(existing);
                let primary_in_f = if mergeable {
                    tx.prepare_cached("SELECT file_id FROM symbols WHERE id = ?1")?
                        .query_row([existing], |r| r.get::<_, i64>(0))?
                        == file_id
                } else {
                    true
                };
                // Only the primary site owns the symbols-row position/signature;
                // a mergeable symbol whose primary is in another file just gains
                // a location here.
                if primary_in_f {
                    update_survivor_row(tx, existing, pf, idx, origin)?;
                }
                upsert_location(tx, existing, file_id, sym)?;
                if mergeable {
                    mergeable_still_in_f.insert(existing);
                } else {
                    matched_nonmerge.insert(existing);
                }
                existing
            }
            _ => {
                let new_id = insert_one_symbol(tx, file_id, pf, idx, origin, arena)?;
                upsert_location(tx, new_id, file_id, sym)?;
                report.new_symbol_names.insert(sym.name.clone());
                if mergeable {
                    mergeable_still_in_f.insert(new_id);
                }
                new_id
            }
        };
        id_by_idx[idx] = id;
        symbol_id_map.insert((pf.path.clone(), sym.qualified_name.clone()), id);
    }

    // --- Vanished non-mergeable rows: full delete (cascade drops their edges) ---
    let mut vanished: Vec<i64> = all_local_nonmerge
        .into_iter()
        .filter(|id| !matched_nonmerge.contains(id))
        .collect();

    // --- Mergeable rows that lost their F declaration: unlink / re-home / delete ---
    let to_unlink: Vec<i64> = merge_located_ids
        .iter()
        .copied()
        .filter(|id| !mergeable_still_in_f.contains(id))
        .collect();
    for id in to_unlink {
        tx.prepare_cached("DELETE FROM symbol_locations WHERE symbol_id = ?1 AND file_id = ?2")?
            .execute(rusqlite::params![id, file_id])?;
        let remaining: Vec<(i64, i64, i64)> = {
            let mut stmt = tx.prepare_cached(
                "SELECT file_id, line, col FROM symbol_locations WHERE symbol_id = ?1 ORDER BY file_id",
            )?;
            let rows = stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        if remaining.is_empty() {
            // No declaration left anywhere — the logical symbol is gone.
            vanished.push(id);
        } else {
            // Survives elsewhere; if F was the primary site, promote another.
            let cur_file: i64 = tx
                .prepare_cached("SELECT file_id FROM symbols WHERE id = ?1")?
                .query_row([id], |r| r.get(0))?;
            if cur_file == file_id {
                let (nf, nl, nc) = remaining[0];
                tx.prepare_cached("UPDATE symbols SET file_id = ?1, line = ?2, col = ?3 WHERE id = ?4")?
                    .execute(rusqlite::params![nf, nl, nc, id])?;
            }
        }
    }

    // Capture the dependents of vanished symbols BEFORE deleting them — the
    // delete cascade-drops their inbound edges, which is exactly what we need to
    // read here.
    if !vanished.is_empty() {
        capture_vanished_dependents(tx, &vanished, &pf.path, report)?;
        let mut del = tx.prepare_cached("DELETE FROM symbols WHERE id = ?1")?;
        for id in &vanished {
            del.execute([*id])?;
        }
    }

    // Survivors keep their id, so the cascade no longer clears their stale
    // OUTGOING refs when the body changes. Clear them explicitly; re-resolution
    // rebuilds the current set. Inbound edges to survivors are untouched.
    clear_outgoing_refs(tx, &id_by_idx)?;

    write_containment(tx, pf, &id_by_idx)?;
    Ok(())
}

/// Update the mutable columns of a survivor (everything the key does not
/// pin: position, signature, doc, visibility, origin). Name/qname/kind are
/// part of the key, so they are stable by construction.
fn update_survivor_row(
    tx: &rusqlite::Transaction<'_>,
    id: i64,
    pf: &ParsedFile,
    idx: usize,
    origin: &str,
) -> Result<()> {
    let sym = &pf.symbols[idx];
    let origin_language: Option<&str> = pf
        .symbol_origin_languages
        .get(idx)
        .and_then(|o| o.as_deref());
    tx.prepare_cached(
        "UPDATE symbols SET line = ?2, col = ?3, end_line = ?4, end_col = ?5,
           scope_path = ?6, signature = ?7, doc_comment = ?8, visibility = ?9,
           origin = ?10, origin_language = ?11
         WHERE id = ?1",
    )
    .context("Failed to prepare survivor update")?
    .execute(rusqlite::params![
        id,
        sym.start_line as i64,
        sym.start_col as i64,
        sym.end_line as i64,
        sym.end_col as i64,
        sym.scope_path,
        sym.signature,
        sym.doc_comment,
        sym.visibility.as_ref().map(|v| v.as_str().to_string()),
        origin,
        origin_language,
    ])?;
    Ok(())
}

/// Insert one symbol (a new key) and return its id. Shares `push_symbol_params`
/// so the column values match the batched full-index path exactly.
fn insert_one_symbol(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
    idx: usize,
    origin: &str,
    arena: Option<&TypeArena>,
) -> Result<i64> {
    let mut params: Vec<Value> = Vec::with_capacity(SYMBOL_COLS);
    push_symbol_params(&mut params, file_id, pf, idx, origin, arena);
    let id: i64 = tx
        .prepare_cached(&symbol_insert_sql(1))
        .context("Failed to prepare single-symbol insert")?
        .query_row(rusqlite::params_from_iter(params.iter()), |r| r.get(0))
        .context("Failed to insert new symbol")?;
    Ok(id)
}

/// Insert or refresh the declaration site for `(symbol_id, file_id)`.
fn upsert_location(
    tx: &rusqlite::Transaction<'_>,
    symbol_id: i64,
    file_id: i64,
    sym: &crate::types::ExtractedSymbol,
) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO symbol_locations (symbol_id, file_id, line, col) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(symbol_id, file_id) DO UPDATE SET line = excluded.line, col = excluded.col",
    )
    .context("Failed to prepare symbol_locations upsert")?
    .execute(rusqlite::params![symbol_id, file_id, sym.start_line as i64, sym.start_col as i64])?;
    Ok(())
}

/// Record the files that hold an edge into any vanished symbol. These are the
/// only dependents that must re-resolve (the vanished targets disappear from
/// their reach); the rewritten file itself is excluded.
fn capture_vanished_dependents(
    tx: &rusqlite::Transaction<'_>,
    vanished: &[i64],
    self_path: &str,
    report: &mut SurvivorReport,
) -> Result<()> {
    tx.execute(
        "CREATE TEMP TABLE IF NOT EXISTS _vanished (id INTEGER PRIMARY KEY)",
        [],
    )?;
    tx.execute("DELETE FROM _vanished", [])?;
    {
        let mut ins = tx.prepare_cached("INSERT OR IGNORE INTO _vanished (id) VALUES (?1)")?;
        for id in vanished {
            ins.execute([*id])?;
        }
    }
    let mut stmt = tx.prepare_cached(
        "SELECT DISTINCT f.path
         FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files   f ON s.file_id = f.id
         JOIN _vanished v ON v.id = e.target_id",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        let path = row?;
        if path != self_path {
            report.vanished_dependent_paths.insert(path);
        }
    }
    tx.execute("DELETE FROM _vanished", [])?;
    Ok(())
}

/// Clear the outgoing edges / external / unresolved refs of the given symbols
/// (file F's survivors + new rows). New rows have none; survivors' stale set is
/// dropped so re-resolution writes the current set.
fn clear_outgoing_refs(tx: &rusqlite::Transaction<'_>, ids: &[i64]) -> Result<()> {
    if ids.iter().all(|id| *id == 0) {
        return Ok(());
    }
    tx.execute(
        "CREATE TEMP TABLE IF NOT EXISTS _fsyms (id INTEGER PRIMARY KEY)",
        [],
    )?;
    tx.execute("DELETE FROM _fsyms", [])?;
    {
        let mut ins = tx.prepare_cached("INSERT OR IGNORE INTO _fsyms (id) VALUES (?1)")?;
        for id in ids {
            if *id != 0 {
                ins.execute([*id])?;
            }
        }
    }
    tx.execute(
        "DELETE FROM edges WHERE source_id IN (SELECT id FROM _fsyms)",
        [],
    )?;
    tx.execute(
        "DELETE FROM unresolved_refs WHERE source_id IN (SELECT id FROM _fsyms)",
        [],
    )?;
    tx.execute(
        "DELETE FROM external_refs WHERE source_id IN (SELECT id FROM _fsyms)",
        [],
    )?;
    tx.execute("DELETE FROM _fsyms", [])?;
    Ok(())
}

/// Write the intra-file containment edge (`parent_index → containing_id`).
/// `id_by_idx[k]` is the row id of `pf.symbols[k]`.
fn write_containment(
    tx: &rusqlite::Transaction<'_>,
    pf: &ParsedFile,
    id_by_idx: &[i64],
) -> Result<()> {
    let mut upd = tx
        .prepare_cached("UPDATE symbols SET containing_id = ?1 WHERE id = ?2")
        .context("Failed to prepare containing_id update")?;
    for (i, sym) in pf.symbols.iter().enumerate() {
        if let Some(p) = sym.parent_index {
            if p < id_by_idx.len() && id_by_idx[p] != 0 && id_by_idx[i] != 0 {
                upd.execute(rusqlite::params![id_by_idx[p], id_by_idx[i]])?;
            }
        }
    }
    Ok(())
}

/// Insert the extract-time routes for a file (INSERT OR IGNORE; the unique
/// index dedups). Handler symbol ids come from the just-written
/// `symbol_id_map`, so they point at survivor ids on the incremental path.
fn insert_routes_incremental(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
    symbol_id_map: &SymbolIdMap,
) -> Result<()> {
    for route in &pf.routes {
        let sym_id = symbol_id_map
            .get(&(
                pf.path.clone(),
                pf.symbols
                    .get(route.handler_symbol_index)
                    .map(|s| s.qualified_name.clone())
                    .unwrap_or_default(),
            ))
            .copied();

        tx.prepare_cached(
            "INSERT OR IGNORE INTO routes
               (file_id, symbol_id, http_method, route_template, resolved_route, line)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
        )
        .context("Failed to prepare route insert")?
        .execute(rusqlite::params![
            file_id,
            sym_id,
            route.http_method,
            route.template,
            pf.symbols.get(route.handler_symbol_index).map(|s| s.start_line),
        ])
        .with_context(|| format!("Failed to insert route for {}", pf.path))?;
    }
    Ok(())
}

/// Write a single `ParsedFile` inside an existing transaction and return its
/// assigned `file_id`. Appends rows into `symbol_id_map` for every symbol.
///
/// Used by the streaming parse pipeline in `full.rs` so files can be
/// persisted one at a time as parser workers produce them, instead of
/// holding every ParsedFile in memory until a single batched write.
///
/// Caller is responsible for transaction lifecycle (begin + commit) and
/// for invalidating any query cache once all writes are done.
pub fn write_one_parsed_file(
    tx: &rusqlite::Transaction<'_>,
    pf: &ParsedFile,
    origin: &str,
    now: i64,
    symbol_id_map: &mut SymbolIdMap,
    is_full: bool,
    arena: Option<&TypeArena>,
) -> Result<i64> {
    // Upsert file row and capture the assigned id via RETURNING.
    let file_id: i64 = tx
        .prepare_cached(
            "INSERT INTO files (path, hash, language, last_indexed, mtime, size, package_id, origin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(path) DO UPDATE SET
               hash = excluded.hash,
               language = excluded.language,
               last_indexed = excluded.last_indexed,
               mtime = excluded.mtime,
               size = excluded.size,
               package_id = excluded.package_id,
               origin = excluded.origin
             RETURNING id",
        )
        .context("Failed to prepare file upsert")?
        .query_row(
            rusqlite::params![pf.path, pf.content_hash, pf.language, now, pf.mtime, pf.size as i64, pf.package_id, origin],
            |r| r.get(0),
        )
        .with_context(|| format!("Failed to upsert file {}", pf.path))?;

    // On a full index the symbols / imports tables were just DROP+CREATE'd
    // in `full.rs`, so these per-file DELETEs are no-ops — but a no-op
    // DELETE is still a round-trip through rusqlite + SQLite's statement
    // executor. Across ~1M files this is tens of seconds of wall-clock.
    // Incremental callers still need the DELETE to clear stale rows.
    if !is_full {
        tx.prepare_cached("DELETE FROM symbols WHERE file_id = ?1")
            .context("Failed to prepare symbol delete")?
            .execute([file_id])?;
        tx.prepare_cached("DELETE FROM imports WHERE file_id = ?1")
            .context("Failed to prepare import delete")?
            .execute([file_id])?;
    }

    insert_symbols_batched(tx, file_id, pf, origin, symbol_id_map, arena)?;

    for route in &pf.routes {
        let sym_id = symbol_id_map
            .get(&(
                pf.path.clone(),
                pf.symbols
                    .get(route.handler_symbol_index)
                    .map(|s| s.qualified_name.clone())
                    .unwrap_or_default(),
            ))
            .copied();

        tx.prepare_cached(
            "INSERT OR IGNORE INTO routes
               (file_id, symbol_id, http_method, route_template, resolved_route, line)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
        )
        .context("Failed to prepare route insert")?
        .execute(rusqlite::params![
            file_id,
            sym_id,
            route.http_method,
            route.template,
            pf.symbols.get(route.handler_symbol_index).map(|s| s.start_line),
        ])
        .with_context(|| format!("Failed to insert route for {}", pf.path))?;
    }

    insert_imports_batched(tx, file_id, pf)?;

    Ok(file_id)
}

/// Origin-aware variant. Callers that index external dependency sources
/// (Go module cache, node_modules, site-packages, etc.) pass "external" so
/// the rows can be partitioned from project code in user-facing queries.
pub fn write_parsed_files_with_origin(
    db: &Database,
    parsed: &[ParsedFile],
    origin: &str,
    arena: Option<&TypeArena>,
) -> Result<(FileIdMap, SymbolIdMap)> {
    // Default to the full-index fast path (tables are fresh after
    // DROP+CREATE). Call sites that re-write over existing rows use the
    // `_incremental` variant, which keeps the per-file DELETE cleanup.
    write_parsed_files_with_origin_impl(db, parsed, origin, /*is_full*/ true, arena)
}

/// Incremental-safe variant: keeps per-file DELETE from symbols/imports so
/// stale rows are removed when a file is re-indexed.
pub fn write_parsed_files_with_origin_incremental(
    db: &Database,
    parsed: &[ParsedFile],
    origin: &str,
    arena: Option<&TypeArena>,
) -> Result<(FileIdMap, SymbolIdMap)> {
    write_parsed_files_with_origin_impl(db, parsed, origin, /*is_full*/ false, arena)
}

fn write_parsed_files_with_origin_impl(
    db: &Database,
    parsed: &[ParsedFile],
    origin: &str,
    is_full: bool,
    arena: Option<&TypeArena>,
) -> Result<(FileIdMap, SymbolIdMap)> {
    let conn = db.conn();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin transaction")?;

    let mut file_id_map: FileIdMap = HashMap::new();
    let mut symbol_id_map: SymbolIdMap = HashMap::new();

    for pf in parsed {
        // Upsert file row and capture the assigned id via RETURNING.
        let file_id: i64 = tx
            .prepare_cached(
                "INSERT INTO files (path, hash, language, last_indexed, mtime, size, package_id, origin)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(path) DO UPDATE SET
                   hash = excluded.hash,
                   language = excluded.language,
                   last_indexed = excluded.last_indexed,
                   mtime = excluded.mtime,
                   size = excluded.size,
                   package_id = excluded.package_id,
                   origin = excluded.origin
                 RETURNING id",
            )
            .context("Failed to prepare file upsert")?
            .query_row(
                rusqlite::params![pf.path, pf.content_hash, pf.language, now, pf.mtime, pf.size as i64, pf.package_id, origin],
                |r| r.get(0),
            )
            .with_context(|| format!("Failed to upsert file {}", pf.path))?;

        file_id_map.insert(pf.path.clone(), file_id);

        // On a full index the tables were just DROP+CREATE'd in full.rs so
        // these per-file DELETEs are no-ops. Skipping them saves ~2 SQL
        // round-trips per file (tens of seconds on 500k+ files).
        if !is_full {
            // Delete existing symbols (ON CONFLICT upsert doesn't cascade-delete).
            tx.prepare_cached("DELETE FROM symbols WHERE file_id = ?1")
                .context("Failed to prepare symbol delete")?
                .execute([file_id])?;

            // Delete existing imports (not cascaded by symbols delete).
            tx.prepare_cached("DELETE FROM imports WHERE file_id = ?1")
                .context("Failed to prepare import delete")?
                .execute([file_id])?;
        }

        // Sub-extracted symbols carry their own origin language (e.g. TS
        // inside a .vue file). Host-extracted symbols use the file's
        // language — represented as NULL in the column for storage
        // efficiency and queryability ("WHERE origin_language IS NOT NULL"
        // yields only spliced multi-language symbols).
        insert_symbols_batched(&tx, file_id, pf, origin, &mut symbol_id_map, arena)?;

        // Insert route records (ASP.NET [HttpGet], [Route], etc.).
        for route in &pf.routes {
            let sym_id = symbol_id_map
                .get(&(
                    pf.path.clone(),
                    pf.symbols
                        .get(route.handler_symbol_index)
                        .map(|s| s.qualified_name.clone())
                        .unwrap_or_default(),
                ))
                .copied();

            // `resolved_route` defaults to `route_template` at extract time —
            // connectors that know about controller-prefix / mount-path joining
            // later overwrite the resolved path with the full concatenation.
            // Previously a post-parse `UPDATE routes SET resolved_route =
            // route_template WHERE resolved_route IS NULL` did this; writing
            // it inline here drops one SQL round-trip per full reindex.
            tx.prepare_cached(
                "INSERT OR IGNORE INTO routes
                   (file_id, symbol_id, http_method, route_template, resolved_route, line)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            )
            .context("Failed to prepare route insert")?
            .execute(rusqlite::params![
                file_id,
                sym_id,
                route.http_method,
                route.template,
                pf.symbols.get(route.handler_symbol_index).map(|s| s.start_line),
            ])
            .with_context(|| format!("Failed to insert route for {}", pf.path))?;
        }

        // Insert import records.
        insert_imports_batched(&tx, file_id, pf)?;
    }

    tx.commit()
        .context("Failed to commit file/symbol transaction")?;

    // Invalidate query caches — symbols changed.
    if let Some(ref cache) = db.query_cache {
        cache.invalidate_all();
    }

    Ok((file_id_map, symbol_id_map))
}

// ---------------------------------------------------------------------------
// FTS content indexing
// ---------------------------------------------------------------------------

/// Update the FTS5 trigram content index for parsed files.
///
/// For incremental: deletes old entries for files being re-indexed,
/// then inserts current content.
pub fn update_fts_content(
    db: &Database,
    parsed: &[ParsedFile],
    file_id_map: &FileIdMap,
) -> Result<u32> {
    let conn = db.conn();
    let mut count = 0u32;

    // For incremental updates, clean up old FTS entries first.
    // For full index after DROP+CREATE this is a no-op (table is empty).
    for pf in parsed {
        if let Some(&file_id) = file_id_map.get(&pf.path) {
            let _ = conn.execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
        }
    }

    // Batch-insert using the content_index module when available.
    let content_entries: Vec<(i64, &str, &str)> = parsed
        .iter()
        .filter_map(|pf| {
            let file_id = file_id_map.get(&pf.path)?;
            let content = pf.content.as_deref()?;
            Some((*file_id, pf.path.as_str(), content))
        })
        .collect();

    match crate::search::content_index::batch_index_content(conn, &content_entries) {
        Ok(n) => count = n as u32,
        Err(e) => warn!("FTS5 content indexing failed: {e}"),
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Code chunking (for embedding/vector search)
// ---------------------------------------------------------------------------

/// Chunk parsed files for embedding and store in `code_chunks`.
///
/// When `is_full` is true (full index after DROP+CREATE), uses the bulk insert
/// path: computes all chunks in memory, batch-inserts in one transaction, skips
/// dedup entirely.  This avoids 50k individual queries on an empty table.
///
/// When `is_full` is false (incremental), uses per-file hash-based dedup to
/// preserve existing vectors for unchanged chunks.
pub fn update_chunks(
    db: &Database,
    parsed: &[ParsedFile],
    file_id_map: &FileIdMap,
    is_full: bool,
) -> Result<u32> {
    let conn = db.conn();

    if is_full {
        // Bulk path: no dedup, no cleanup, one transaction.
        let files: Vec<(i64, &str)> = parsed
            .iter()
            .filter_map(|pf| {
                let file_id = file_id_map.get(&pf.path)?;
                let content = pf.content.as_deref()?;
                Some((*file_id, content))
            })
            .collect();

        match crate::search::chunker::bulk_chunk_and_store(conn, &files) {
            Ok(n) => Ok(n),
            Err(e) => {
                warn!("Bulk chunking failed: {e}");
                Ok(0)
            }
        }
    } else {
        // Incremental path: per-file dedup preserves existing vectors.
        let mut total = 0u32;
        for pf in parsed {
            if let (Some(&file_id), Some(content)) =
                (file_id_map.get(&pf.path), pf.content.as_deref())
            {
                let _ = crate::search::vector_store::delete_file_vectors(conn, file_id);
                let _ = conn.execute("DELETE FROM code_chunks WHERE file_id = ?1", [file_id]);

                match crate::search::chunker::chunk_and_store(conn, file_id, content) {
                    Ok(n) => total += n,
                    Err(e) => debug!("Failed to chunk {}: {e}", pf.path),
                }
            }
        }
        Ok(total)
    }
}

// ---------------------------------------------------------------------------
// File deletion
// ---------------------------------------------------------------------------

/// Delete files from the index by relative path.
///
/// Handles CASCADE-covered tables (symbols, edges, etc.) via the FK
/// constraint, plus virtual tables (vec_chunks, fts_content, flow_edges)
/// that require manual cleanup.
///
/// All per-file DELETE statements run inside a single transaction so the
/// database is never left in a partially-deleted state if the process is
/// interrupted mid-batch.
pub fn delete_files(db: &Database, paths: &[String]) -> Result<Vec<(i64, String)>> {
    let conn = db.conn();
    let mut deleted = Vec::new();

    if paths.is_empty() {
        return Ok(deleted);
    }

    // Resolve file IDs outside the transaction (read-only).
    let mut file_ids: Vec<(i64, &String)> = Vec::with_capacity(paths.len());
    for rel_path in paths {
        if let Ok(file_id) = conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            [rel_path.as_str()],
            |r| r.get::<_, i64>(0),
        ) {
            file_ids.push((file_id, rel_path));
        }
    }

    if file_ids.is_empty() {
        return Ok(deleted);
    }

    // Virtual-table cleanup must happen before the transaction DELETEs the
    // rows that the virtual tables reference.  sqlite-vec is not transactional
    // in the same sense, so we clean it up first while the rows still exist.
    for (file_id, _) in &file_ids {
        let _ = crate::search::vector_store::delete_file_vectors(conn, *file_id);
    }

    // Wrap all DELETE statements in one transaction.
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin delete transaction")?;

    for (file_id, rel_path) in &file_ids {
        // CASCADE handles symbols, edges, imports, routes, code_chunks, etc.
        tx.execute("DELETE FROM files WHERE id = ?1", [file_id])?;

        // Manual cleanup for tables without FK to files.
        let _ = tx.execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
        let _ = tx.execute(
            "DELETE FROM flow_edges WHERE source_file_id = ?1 OR target_file_id = ?1",
            [file_id],
        );

        deleted.push((*file_id, (*rel_path).clone()));
        debug!("Deleted file from index: {rel_path}");
    }

    tx.commit().context("Failed to commit delete transaction")?;

    Ok(deleted)
}

// ---------------------------------------------------------------------------
// Package write/detect
// ---------------------------------------------------------------------------

/// Write detected packages to the `packages` table and return them with IDs assigned.
///
/// Existing packages (matched by path) are updated; new ones are inserted.
/// Returns the full list with `id` populated.
pub fn write_packages(
    db: &Database,
    packages: &[crate::types::PackageInfo],
) -> Result<Vec<crate::types::PackageInfo>> {
    let conn = db.conn();
    let mut result = Vec::with_capacity(packages.len());

    for pkg in packages {
        // Composite identity is (path, kind); kind is NOT NULL in the new
        // schema. Old detectors that left kind unset get bucketed as
        // 'unknown' so the conflict target stays well-defined.
        let kind_value = pkg.kind.clone().unwrap_or_else(|| "unknown".to_string());
        let id: i64 = conn
            .prepare_cached(
                "INSERT INTO packages (name, path, kind, manifest, declared_name, is_publishable)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path, kind) DO UPDATE SET
                   name = excluded.name,
                   manifest = excluded.manifest,
                   declared_name = excluded.declared_name,
                   is_publishable = excluded.is_publishable
                 RETURNING id",
            )?
            .query_row(
                rusqlite::params![
                    pkg.name,
                    pkg.path,
                    kind_value,
                    pkg.manifest,
                    pkg.declared_name,
                    pkg.is_publishable as i64,
                ],
                |r| r.get(0),
            )
            .with_context(|| format!("Failed to upsert package {} ({})", pkg.name, kind_value))?;

        result.push(crate::types::PackageInfo {
            id: Some(id),
            name: pkg.name.clone(),
            path: pkg.path.clone(),
            kind: Some(kind_value),
            manifest: pkg.manifest.clone(),
            declared_name: pkg.declared_name.clone(),
            is_publishable: pkg.is_publishable,
        });
    }

    // Remove packages that are no longer detected. Composite key means we
    // delete by (path, kind) tuples — a path may legitimately be re-used
    // across ecosystems (Tauri root: cargo + npm).
    if !packages.is_empty() {
        let kind_buf: Vec<String> = packages
            .iter()
            .map(|p| p.kind.clone().unwrap_or_else(|| "unknown".to_string()))
            .collect();
        let tuples: String = (1..=packages.len())
            .map(|i| format!("(?{}, ?{})", i * 2 - 1, i * 2))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "DELETE FROM packages WHERE (path, kind) NOT IN (VALUES {tuples})"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut params: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(packages.len() * 2);
        for (pkg, kind) in packages.iter().zip(kind_buf.iter()) {
            params.push(&pkg.path as &dyn rusqlite::types::ToSql);
            params.push(kind as &dyn rusqlite::types::ToSql);
        }
        stmt.execute(params.as_slice())?;
    } else {
        // No packages detected this run — clear all stale rows.
        conn.execute("DELETE FROM packages", [])?;
    }

    Ok(result)
}

/// Assign `package_id` to each parsed file based on longest path-prefix match.
pub fn assign_package_ids(
    parsed: &mut [crate::types::ParsedFile],
    packages: &[crate::types::PackageInfo],
) {
    if packages.is_empty() {
        return;
    }
    // Sort packages by path length descending for longest-prefix-first matching.
    // Tie-break by `kind` then `id` so two packages at the same path (Tauri
    // root with both Cargo.toml and package.json, for example) win in a
    // deterministic order. Without the tie-break the same root file could
    // land on different packages across runs.
    let mut sorted: Vec<&crate::types::PackageInfo> = packages.iter().collect();
    sorted.sort_by(|a, b| {
        b.path
            .len()
            .cmp(&a.path.len())
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.id.cmp(&b.id))
    });

    for pf in parsed.iter_mut() {
        for pkg in &sorted {
            // Normalize separators for comparison.
            let file_path = pf.path.replace('\\', "/");
            let pkg_path = pkg.path.replace('\\', "/");
            if pkg_path.is_empty() {
                // Root package: any file inside the project belongs to it.
                // Sort order (length desc) ensures this only fires when no
                // proper-prefix package matched first. Mirrors
                // `package_id_for_path` in `full.rs`.
                pf.package_id = pkg.id;
                break;
            }
            if file_path.starts_with(&pkg_path)
                && (file_path.len() == pkg_path.len()
                    || file_path.as_bytes()[pkg_path.len()] == b'/')
            {
                pf.package_id = pkg.id;
                break;
            }
        }
    }
}

/// M3: Write per-package dependency declarations to `package_deps`.
///
/// `entries` is a list of `(package_id, ecosystem, dep_name, version, kind)`
/// rows derived from each workspace package's manifest data during
/// `parse_external_sources`. The write is an upsert on the composite
/// primary key `(package_id, ecosystem, dep_name)` — re-running a full
/// index replaces any stale version/kind without leaving duplicates.
///
/// Callers should `DELETE FROM package_deps` first on incremental paths
/// that discover a shrunk manifest; full index drops + recreates the
/// table so no explicit clear is needed there.
pub fn write_package_deps(
    db: &Database,
    entries: &[(i64, &str, String, Option<String>, &'static str)],
) -> Result<usize> {
    if entries.is_empty() {
        return Ok(0);
    }
    let conn = db.conn();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(package_id, ecosystem, dep_name) DO UPDATE SET
           version = excluded.version,
           kind    = excluded.kind",
    )?;
    let mut written = 0usize;
    for (pkg_id, ecosystem, dep_name, version, kind) in entries {
        stmt.execute(rusqlite::params![pkg_id, ecosystem, dep_name, version, kind])?;
        written += 1;
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Package loading (for incremental package_id assignment)
// ---------------------------------------------------------------------------

/// Load all packages from the `packages` table.
///
/// Used during incremental indexing to assign `package_id` to newly parsed
/// files without re-running full package detection.
pub fn load_packages_from_db(db: &Database) -> Result<Vec<crate::types::PackageInfo>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, name, path, kind, manifest, declared_name, is_publishable FROM packages",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(crate::types::PackageInfo {
            id: Some(r.get::<_, i64>(0)?),
            name: r.get::<_, String>(1)?,
            path: r.get::<_, String>(2)?,
            kind: r.get::<_, Option<String>>(3)?,
            manifest: r.get::<_, Option<String>>(4)?,
            declared_name: r.get::<_, Option<String>>(5)?,
            is_publishable: r.get::<_, i64>(6)? != 0,
        })
    })?;
    let mut packages = Vec::new();
    for row in rows {
        packages.push(row?);
    }
    Ok(packages)
}

// ---------------------------------------------------------------------------
// Symbol ID loading (for incremental resolution)
// ---------------------------------------------------------------------------

/// Load the full symbol_id_map from the database.
///
/// Used during incremental resolution so the resolver can see symbols from
/// unchanged files (not just the ones in the current parse batch).
pub fn load_symbol_id_map(db: &Database) -> Result<SymbolIdMap> {
    let mut map = SymbolIdMap::new();
    let mut stmt = db.conn().prepare(
        "SELECT f.path, s.qualified_name, s.id
         FROM symbols s
         JOIN files f ON f.id = s.file_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (path, qname, id) = row?;
        map.insert((path, qname), id);
    }
    Ok(map)
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
