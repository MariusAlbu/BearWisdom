// =============================================================================
// indexer/resolve/engine/index/augment.rs — post-construction additions
//
// Methods that fold more data into an already-built SymbolIndex without
// reconstructing it. Two callers:
//
//   * The full-reindex expand loop adds newly-discovered parsed files via
//     `augment_from_parsed` instead of rebuilding the whole index per
//     iteration.
//   * The incremental indexer reads unchanged-file symbols from SQLite via
//     `augment_from_db` / `augment_from_db_collecting_ids` so cross-file
//     resolution still hits symbols in files the current pass didn't reparse.
//
// Also hosts the small utility methods `take_chain_misses` and
// `set_external_paths`, which mutate index state from outside the build path.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::types::{EdgeKind, ParsedFile, SymbolKind, Visibility};

use super::super::{
    find_matching_bracket, parse_return_type_from_signature, resolve_type_name_in_scope,
};
use super::{common_prefix_len, is_type_like_kind};
use super::SymbolIndex;
use crate::indexer::resolve::engine::{ChainMiss, SymbolInfo, SymbolLookup};

impl SymbolIndex {
    /// Augment an already-built index with symbols from `new_files` —
    /// the in-memory complement of `augment_from_db_collecting_ids`.
    ///
    /// Used by the full-reindex resolve loop: after `expand_chain_reachability`
    /// adds files to `parsed`, this folds those new files' symbols into the
    /// existing index instead of rebuilding from scratch each iteration.
    /// On a 280k-symbol aspnetcore index, build_with_context costs ~5-10s;
    /// running it 8× across the expand loop is ~40-80s of redundant work.
    ///
    /// Runs the essential passes for resolution to keep working on cross-
    /// iteration chain walks:
    ///   - Pass 1: basic indexes (by_name, by_qname, by_file,
    ///     members_by_parent, types_by_name, by_package)
    ///   - Pass 2: signature-derived return types for new files (so chain
    ///     walks hopping through new external method return types resolve)
    ///   - Pass 4: generic_params from signatures
    ///   - Pass 5: inherits_map for new classes
    ///
    /// Skipped (best-effort tradeoff for performance):
    ///   - Variable inference from chain TypeRefs (rare on externals)
    ///   - Re-export map updates (typically settled at initial build)
    pub fn augment_from_parsed(
        &mut self,
        new_files: &[ParsedFile],
        symbol_id_map: &HashMap<(String, String), i64>,
    ) {
        // Pass 1: basic indexes for new files.
        for pf in new_files {
            let file_path: Arc<str> = Arc::from(pf.path.as_str());
            for sym in &pf.symbols {
                let Some(&id) = symbol_id_map.get(&(pf.path.clone(), sym.qualified_name.clone()))
                else {
                    continue;
                };

                let info = SymbolInfo {
                    id,
                    name: sym.name.clone(),
                    qualified_name: sym.qualified_name.clone(),
                    kind: sym.kind.as_str().to_string(),
                    visibility: sym.visibility.as_ref().map(|v| format!("{v:?}").to_lowercase()),
                    file_path: Arc::clone(&file_path),
                    scope_path: sym.scope_path.clone(),
                    package_id: pf.package_id,
                    signature: sym.signature.clone(),
                };

                self.by_name.entry(sym.name.clone()).or_default().push(info.clone());

                match self.by_qname.entry(sym.qualified_name.clone()) {
                    std::collections::btree_map::Entry::Vacant(e) => {
                        e.insert(info.clone());
                    }
                    std::collections::btree_map::Entry::Occupied(occ) => {
                        let entry = self.qname_duplicates
                            .entry(sym.qualified_name.clone())
                            .or_insert_with(|| vec![occ.get().clone()]);
                        entry.push(info.clone());
                    }
                }

                self.by_file
                    .entry(pf.path.clone())
                    .or_default()
                    .push(info.clone());

                let parent_key: &str = match sym.qualified_name.rfind('.') {
                    Some(idx) => &sym.qualified_name[..idx],
                    None => "",
                };
                if is_type_like_kind(&info.kind) {
                    self.types_by_name
                        .entry(sym.name.clone())
                        .or_default()
                        .push(info.clone());
                }
                self.members_by_parent
                    .entry(parent_key.to_string())
                    .or_default()
                    .push(info.clone());

                if let Some(pkg_id) = info.package_id {
                    self.by_package.entry(pkg_id).or_default().push(info.clone());
                }
            }
        }

        // Pass 2: type_info for new files. Reuses the same logic as
        // build_with_context's Pass 2, scoped to new_files. Populates
        // field_type, return_type (incl. signature-parsed for .NET DLL
        // metadata), and generic_params on self.type_info.
        for pf in new_files {
            let mut type_refs_by_sym: Vec<Vec<&str>> = vec![Vec::new(); pf.symbols.len()];
            for r in &pf.refs {
                if r.kind != EdgeKind::TypeRef || r.module.is_some() {
                    continue;
                }
                let idx = r.source_symbol_index;
                if idx < type_refs_by_sym.len() {
                    type_refs_by_sym[idx].push(r.target_name.as_str());
                }
            }

            for (sym_idx, sym) in pf.symbols.iter().enumerate() {
                let type_refs = &type_refs_by_sym[sym_idx];
                match sym.kind {
                    SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable => {
                        if let Some(first) = type_refs.first() {
                            let resolved = resolve_type_name_in_scope(
                                first,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            self.type_info
                                .entry(sym.qualified_name.clone())
                                .or_default()
                                .field_type = Some(resolved);
                            if type_refs.len() > 1 {
                                self.type_info
                                    .entry(sym.qualified_name.clone())
                                    .or_default()
                                    .type_args = type_refs[1..].iter().map(|s| s.to_string()).collect();
                            }
                        }
                    }
                    SymbolKind::TypeAlias => {
                        if let Some(first) = type_refs.first() {
                            self.type_info
                                .entry(sym.qualified_name.clone())
                                .or_default()
                                .field_type = Some(first.to_string());
                        }
                    }
                    SymbolKind::Method | SymbolKind::Function | SymbolKind::Constructor => {
                        if let Some(&last) = type_refs.last() {
                            let resolved = resolve_type_name_in_scope(
                                last,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            self.type_info
                                .entry(sym.qualified_name.clone())
                                .or_default()
                                .return_type = Some(resolved);
                        }
                        // Signature-derived return type fallback for .NET
                        // DLL metadata (no TypeRef refs but signature has type).
                        let already = self.type_info
                            .get(&sym.qualified_name)
                            .and_then(|ti| ti.return_type.as_ref())
                            .is_some();
                        if !already {
                            if let Some(sig) = &sym.signature {
                                if let Some(rt) = parse_return_type_from_signature(sig) {
                                    let resolved = resolve_type_name_in_scope(
                                        &rt,
                                        sym.scope_path.as_deref(),
                                        &self.by_qname,
                                    );
                                    self.type_info
                                        .entry(sym.qualified_name.clone())
                                        .or_default()
                                        .return_type = Some(resolved);
                                }
                            }
                        }
                    }
                    // A class symbol IS the callable — calling `Foo()` returns
                    // an instance of `Foo`. Record `return_type = qualified_name`
                    // so the chain walker can follow `x = Foo(); x.method()`.
                    SymbolKind::Class => {
                        self.type_info
                            .entry(sym.qualified_name.clone())
                            .or_default()
                            .return_type = Some(sym.qualified_name.clone());
                    }
                    _ => {}
                }
            }

            // Generic params from signature for class/interface/struct/etc.
            for sym in &pf.symbols {
                if !matches!(
                    sym.kind,
                    SymbolKind::Class | SymbolKind::Interface | SymbolKind::Struct
                        | SymbolKind::TypeAlias | SymbolKind::Function | SymbolKind::Method
                ) {
                    continue;
                }
                let Some(sig) = &sym.signature else { continue };
                let bracket_pairs: &[(char, char)] = &[('<', '>'), ('[', ']')];
                for &(open, close) in bracket_pairs {
                    if let Some(start) = sig.find(open) {
                        if let Some(relative_end) =
                            find_matching_bracket(&sig[start..], open, close)
                        {
                            let end = start + relative_end;
                            let params_str = &sig[start + 1..end];
                            let params: Vec<String> = params_str
                                .split(',')
                                .map(|s| {
                                    s.trim()
                                        .split(|c: char| c == '[' || c == '<' || c == ':')
                                        .next()
                                        .unwrap_or("")
                                        .split_whitespace()
                                        .next()
                                        .unwrap_or("")
                                        .to_string()
                                })
                                .filter(|s| !s.is_empty())
                                .collect();
                            if !params.is_empty() {
                                self.type_info
                                    .entry(sym.name.clone())
                                    .or_default()
                                    .generic_params = params.clone();
                                self.type_info
                                    .entry(sym.qualified_name.clone())
                                    .or_default()
                                    .generic_params = params;
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Pass 5: inherits_map for new class/interface symbols.
        for pf in new_files {
            for r in &pf.refs {
                if r.kind != EdgeKind::Inherits {
                    continue;
                }
                let Some(child_sym) = pf.symbols.get(r.source_symbol_index) else {
                    continue;
                };
                if !matches!(child_sym.kind, SymbolKind::Class | SymbolKind::Interface) {
                    continue;
                }
                let child_qname = &child_sym.qualified_name;
                if self.inherits_map.contains_key(child_qname) {
                    continue;
                }
                let parent_simple = r.target_name.trim_start_matches('\\');
                let candidates = self.by_name.get(parent_simple).map(|v| v.as_slice()).unwrap_or(&[]);
                if candidates.is_empty() {
                    continue;
                }
                let child_ns = child_qname.rfind('.').map(|i| &child_qname[..i]).unwrap_or("");
                let best = if candidates.len() == 1 {
                    &candidates[0]
                } else {
                    candidates
                        .iter()
                        .max_by_key(|c| {
                            let cns = c.qualified_name.rfind('.').map(|i| &c.qualified_name[..i]).unwrap_or("");
                            common_prefix_len(child_ns, cns)
                        })
                        .unwrap_or(&candidates[0])
                };
                self.inherits_map.insert(child_qname.clone(), best.qualified_name.clone());
            }
        }

        // Pass 6: Angular selector map for new files.
        for pf in new_files {
            for (selector, class_qname) in &pf.component_selectors {
                if !selector.is_empty() && !class_qname.is_empty() {
                    self.angular_selectors.insert(selector.clone(), class_qname.clone());
                }
            }
        }
    }

    /// Drain the chain-walker miss accumulator.
    ///
    /// Called by `resolve_and_write` after the initial resolution pass to
    /// drive R3 lazy-reload via `Ecosystem::resolve_symbol`. Returns the
    /// accumulated bail-outs in insertion order; the buffer is emptied.
    pub fn take_chain_misses(&self) -> Vec<ChainMiss> {
        self.chain_misses
            .lock()
            .expect("chain_misses mutex poisoned")
            .drain(..)
            .collect()
    }

    /// Seed the external-paths set.
    ///
    /// The caller reads this from the DB (`files WHERE origin='external'`)
    /// and installs it before resolution starts. It covers the gap between
    /// "path starts with `ext:`" (the implicit convention used by most
    /// external pipelines) and "file was written with origin='external'
    /// but kept its project-relative path" (specifically the
    /// script-tag-vendor-JS pipeline). Chain walkers consult both signals
    /// via `SymbolLookup::is_external_file`.
    pub fn set_external_paths(&mut self, paths: HashSet<String>) {
        self.external_paths = paths;
    }

    /// Load all symbols from the database into the index, filling gaps left by
    /// an incremental build where only changed files were parsed.
    ///
    /// Symbols already present (from parsed files) are NOT overwritten — the
    /// parsed data is richer (has type info, reexports).  This only adds
    /// entries for symbols in unchanged files so the engine resolver can find
    /// them by name during cross-file resolution.
    ///
    /// Call this AFTER `build_with_context` for incremental resolution.
    pub fn augment_from_db(&mut self, conn: &rusqlite::Connection) {
        let _ = self.augment_from_db_collecting_ids(conn);
    }

    /// Same as `augment_from_db`, but also returns the `(path, qname) → id`
    /// map for every row scanned. The incremental resolve path needs this
    /// map to drive the heuristic resolver — emitting it from the same
    /// SELECT halves the per-save DB I/O on huge indexes (was ~200MB on
    /// aspnetcore between this and the standalone `load_symbol_id_map`).
    pub fn augment_from_db_collecting_ids(
        &mut self,
        conn: &rusqlite::Connection,
    ) -> HashMap<(String, String), i64> {
        let mut id_map: HashMap<(String, String), i64> = HashMap::new();

        let mut stmt = match conn.prepare(
            "SELECT s.id, s.name, s.qualified_name, s.kind, f.path,
                    s.scope_path, s.visibility, f.package_id, s.signature
             FROM symbols s
             JOIN files f ON f.id = s.file_id",
        ) {
            Ok(s) => s,
            Err(_) => return id_map,
        };

        let rows = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        }) {
            Ok(r) => r,
            Err(_) => return id_map,
        };

        for row in rows {
            let Ok((id, name, qname, kind, file_path, scope_path, visibility, package_id, signature)) = row
            else {
                continue;
            };

            // Always collect the (path, qname) → id mapping — heuristic
            // needs full project coverage even for symbols already in the
            // index from `parsed`.
            id_map.insert((file_path.clone(), qname.clone()), id);

            // Skip symbols already indexed from parsed files.
            if self.by_qname.contains_key(&qname) {
                continue;
            }

            // Arc<str> for file_path: shared across all symbols in the same file
            // within this batch; BTreeMap insert gets one clone, by_file gets another.
            let file_arc: Arc<str> = Arc::from(file_path.as_str());

            let info = SymbolInfo {
                id,
                name: name.clone(),
                qualified_name: qname.clone(),
                kind,
                visibility,
                file_path: Arc::clone(&file_arc),
                scope_path,
                package_id,
                signature,
            };

            self.by_name.entry(name.clone()).or_default().push(info.clone());
            self.by_qname.insert(qname.clone(), info.clone());
            if let Some(pkg_id) = info.package_id {
                self.by_package.entry(pkg_id).or_default().push(info.clone());
            }
            let parent_key: String = match qname.rfind('.') {
                Some(idx) => qname[..idx].to_string(),
                None => String::new(),
            };
            if is_type_like_kind(&info.kind) {
                self.types_by_name
                    .entry(name)
                    .or_default()
                    .push(info.clone());
            }
            self.members_by_parent
                .entry(parent_key)
                .or_default()
                .push(info.clone());
            // by_file key stays String (one allocation per file, not per symbol)
            self.by_file.entry(file_path).or_default().push(info);
        }
        id_map
    }
}
