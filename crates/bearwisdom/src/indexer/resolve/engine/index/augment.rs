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
    find_matching_bracket, is_jvm_language, merge_where_bounds, parse_generic_param_clause,
    parse_return_type_from_jvm_descriptor,
    parse_return_type_from_signature, parse_return_type_positional, parse_type_head_and_args,
    resolve_type_name_in_scope,
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

                // Direct-children index keyed on the PARENT symbol's qualified
                // name, resolved structurally via `parent_index` so a child whose
                // own qname dropped a prefix still files under its real parent.
                // Falls back to qname truncation for symbols with no parent
                // pointer; top-level symbols go under "".
                let parent_key: String = match sym.parent_index.and_then(|p| pf.symbols.get(p)) {
                    Some(parent) => parent.qualified_name.clone(),
                    None => match sym.qualified_name.rfind('.') {
                        Some(idx) => sym.qualified_name[..idx].to_string(),
                        None => String::new(),
                    },
                };
                if is_type_like_kind(&info.kind) {
                    self.types_by_name
                        .entry(sym.name.clone())
                        .or_default()
                        .push(info.clone());
                }
                self.members_by_parent
                    .entry(parent_key)
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
                // Module-tagged USAGE TypeRefs feed the per-symbol type maps;
                // only an import STATEMENT's own binding ref is excluded (its
                // `source_symbol_index` is positional, not a type attribution).
                if r.kind != EdgeKind::TypeRef || r.is_import_binding {
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
                    SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable | SymbolKind::Parameter => {
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
                        } else if is_jvm_language(&pf.language) {
                            // A JVM field has no TypeRef (externals emit no refs);
                            // its type lives in the raw bytecode descriptor
                            // signature (`Lcom/foo/Bar;`). Decode that here.
                            if let Some(decoded) = sym
                                .signature
                                .as_deref()
                                .and_then(parse_return_type_from_jvm_descriptor)
                            {
                                let resolved = resolve_type_name_in_scope(
                                    &decoded,
                                    sym.scope_path.as_deref(),
                                    &self.by_qname,
                                );
                                self.type_info
                                    .entry(sym.qualified_name.clone())
                                    .or_default()
                                    .field_type = Some(resolved);
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
                        let sig_rt: Option<String> = sym.signature.as_deref().and_then(|s| {
                            parse_return_type_from_signature(s).or_else(|| {
                                if sym.kind == SymbolKind::Constructor {
                                    None
                                } else {
                                    parse_return_type_positional(s)
                                }
                            })
                            .or_else(|| {
                                // JVM bytecode descriptor (`(params)Ret`, Maven /
                                // `.class` metadata): decode the return element
                                // type. Gated on the JVM language set so it never
                                // perturbs the colon/arrow path.
                                if is_jvm_language(&pf.language) {
                                    parse_return_type_from_jvm_descriptor(s)
                                } else {
                                    None
                                }
                            })
                        });
                        // A signature that parses to a generic application
                        // (`Repository<User>`) yields an unambiguous head + args.
                        // Structural types containing an inner generic (tuples,
                        // unions, function types) produce a non-identifier head
                        // after splitting — those fall to the else branch, so
                        // the head must be a bare/dotted name.
                        let sig_generic: Option<(String, Vec<String>)> =
                            sig_rt.as_deref().and_then(|rt| {
                                let (head, args) = parse_type_head_and_args(rt);
                                let head_is_name = !head.is_empty()
                                    && head
                                        .chars()
                                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
                                if args.is_empty() || !head_is_name {
                                    None
                                } else {
                                    Some((
                                        head.to_string(),
                                        args.iter().map(|s| s.to_string()).collect(),
                                    ))
                                }
                            });
                        if let Some((head, args)) = sig_generic {
                            let resolved = resolve_type_name_in_scope(
                                &head,
                                sym.scope_path.as_deref(),
                                &self.by_qname,
                            );
                            let ti = self.type_info.entry(sym.qualified_name.clone()).or_default();
                            ti.return_type = Some(resolved);
                            ti.return_type_args = args;
                        } else {
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
                            let already = self
                                .type_info
                                .get(&sym.qualified_name)
                                .and_then(|ti| ti.return_type.as_ref())
                                .is_some();
                            if !already {
                                if let Some(rt) = &sig_rt {
                                    let resolved = resolve_type_name_in_scope(
                                        rt,
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
                            let mut parsed = parse_generic_param_clause(&sig[start + 1..end]);
                            merge_where_bounds(&mut parsed, sig);
                            if !parsed.is_empty() {
                                let (params, bounds): (Vec<String>, Vec<Option<String>>) =
                                    parsed.into_iter().unzip();
                                for key in [&sym.name, &sym.qualified_name] {
                                    let ti = self.type_info.entry(key.clone()).or_default();
                                    ti.generic_params = params.clone();
                                    ti.generic_param_bounds = bounds.clone();
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Pass 5: inherits_map for new class/interface/trait/struct symbols.
        for pf in new_files {
            for r in &pf.refs {
                if r.kind != EdgeKind::Inherits {
                    continue;
                }
                let Some(child_sym) = pf.symbols.get(r.source_symbol_index) else {
                    continue;
                };
                if !matches!(
                    child_sym.kind,
                    SymbolKind::Class | SymbolKind::Interface | SymbolKind::Trait | SymbolKind::Struct
                ) {
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

        // Pass 7: intern newly-populated string-typed type_info entries into
        // the workspace TypeArena. Idempotent: only fills slots whose TypeId
        // companion is still empty. Mirrors the post-merge intern pass in
        // `build_with_context`, including structural decomposition for
        // generic applications via `intern_type_str`.
        let arena = &self.type_arena;
        for ti in self.type_info.values_mut() {
            if ti.field_type_id.is_none() {
                if let Some(ft) = ti.field_type.as_deref() {
                    if !ft.is_empty() {
                        ti.field_type_id = Some(arena.intern_type_str(ft));
                    }
                }
            }
            if ti.return_type_id.is_none() {
                if let Some(rt) = ti.return_type.as_deref() {
                    if !rt.is_empty() {
                        ti.return_type_id = Some(arena.intern_type_str(rt));
                    }
                }
            }
            if ti.type_arg_ids.is_empty() && !ti.type_args.is_empty() {
                ti.type_arg_ids = ti
                    .type_args
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| arena.intern_type_str(s))
                    .collect();
            }
            if ti.return_type_arg_ids.is_empty() && !ti.return_type_args.is_empty() {
                ti.return_type_arg_ids = ti
                    .return_type_args
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| arena.intern_type_str(s))
                    .collect();
            }
        }

        // Final sweep: derive strings from canonical TypeIds so the legacy
        // string accessors always reflect the TypeId-typed source of truth.
        // generic_params get their TypeId companions interned through
        // arena.intern_generic for substitution consumers.
        for ti in self.type_info.values_mut() {
            if let Some(id) = ti.field_type_id {
                ti.field_type = Some(arena.format_type(id));
            }
            if let Some(id) = ti.return_type_id {
                ti.return_type = Some(arena.format_type(id));
            }
            if !ti.type_arg_ids.is_empty() {
                ti.type_args = ti
                    .type_arg_ids
                    .iter()
                    .map(|id| arena.format_type(*id))
                    .collect();
            }
            if !ti.return_type_arg_ids.is_empty() {
                ti.return_type_args = ti
                    .return_type_arg_ids
                    .iter()
                    .map(|id| arena.format_type(*id))
                    .collect();
            }
        }

        for (key, ti) in self.type_info.iter_mut() {
            if !ti.generic_param_type_ids.is_empty() || ti.generic_params.is_empty() {
                continue;
            }
            let owner_id = self.by_qname.get(key).map(|info| info.id).unwrap_or(0) as usize;
            ti.generic_param_type_ids = ti
                .generic_params
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let bound = ti
                        .generic_param_bounds
                        .get(i)
                        .and_then(|b| b.as_deref())
                        .map(|b| arena.intern_type_str(b));
                    let param = arena.intern_generic(
                        crate::type_checker::core::types::GenericParamData {
                            name: name.clone(),
                            owner_symbol_index: owner_id,
                            bound,
                        },
                    );
                    arena.intern(crate::type_checker::core::types::Type::Generic { param })
                })
                .collect();
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

    /// Gap-fill an inferred return type for `qname` (INFER-3 / INFER-2). Only
    /// fills when the function has NO existing return type — an inferred return
    /// never overrides a declared or signature-derived one (soundness: declared
    /// wins). Returns `true` only when it newly filled a gap, which the
    /// orchestrator uses to drive the inference fixpoint to convergence: once
    /// every inferable return is filled, a pass sets nothing new and the loop
    /// stops.
    pub fn set_inferred_return(&mut self, qname: String, ty: String) -> bool {
        let entry = self.type_info.entry(qname).or_default();
        if entry.return_type.is_none() {
            entry.return_type = Some(ty);
            true
        } else {
            false
        }
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
