//! Source-bound signatures overwrite only their exact declaration's type slots.
use super::*;

impl Compilation {
    pub(in crate::indexer::resolve::engine) fn intrinsic_member_type(
        &self,
        kind: crate::type_checker::core::types::Intrinsic,
    ) -> Option<crate::type_checker::core::types::TypeId> {
        let row = self.program_views.legacy_intrinsic(kind)?;
        (self as &dyn SymbolLookup).declaration_type(&self.arena, row)
    }
    pub(in crate::indexer::resolve::engine) fn program_lookup(
        &self,
        path: &str,
    ) -> Option<super::super::program_view::Lookup<'_>> {
        self.program_views.for_file(self, path)
    }
    pub(in crate::indexer::resolve::engine) fn source_program_lookup(
        &self,
        file: &ParsedFile,
    ) -> Option<super::super::program_view::Lookup<'_>> {
        self.program_views.for_source(self, file)
    }
    fn rebuild_program_views(&mut self) {
        self.program_views =
            super::super::program_view::Store::build(&self.modules, self, &self.arena);
    }
    pub(in crate::indexer::resolve::engine) fn namespace_use(
        &self,
        file: &str,
        usage: crate::indexer::namespaces::Use,
    ) -> Option<super::super::contract::flow_cache::LocalReference> {
        super::super::namespace_input::use_fact(&self.modules, self, file, usage)
    }
    pub(super) fn refresh_module_bindings(&mut self, files: &[ParsedFile], ids: &SymbolIds) {
        self.capture_source_parameters(files, ids);
        let mut modules = std::mem::take(&mut self.modules);
        for file in files {
            if let Some(mut input) = super::super::module_input::capture(file, ids) {
                if let Some(globals) = &mut input.globals {
                    globals.types = Some(super::super::program_types::capture(
                        file,
                        ids,
                        self,
                        &self.arena,
                    ));
                }
                input
                    .signatures
                    .extend(super::super::module_type_inputs::capture(
                        file,
                        ids,
                        self,
                        &self.arena,
                    ));
                input.bases = super::super::base_receiver::capture(file, ids, self, &self.arena);
                input.traits =
                    super::super::module_trait_inputs::capture(file, ids, self, &self.arena);
                modules.inputs.insert(input.path.clone(), input);
            }
        }
        modules.rebuild(self);
        self.modules = modules;
    }

    fn capture_source_parameters(&mut self, files: &[ParsedFile], ids: &SymbolIds) {
        for file in files {
            let (Some(graph), Some(source)) = (&file.flow.lexical, &file.flow.namespaces) else {
                continue;
            };
            for (&slot, names) in &graph.types.generic_declarations {
                let Some(id) = ids.row_id(&file.path, slot) else {
                    continue;
                };
                let info = self.type_info_by_id.entry(id).or_default();
                // Positions under this declaration are identities. No spelling
                // equality and no search through a signature or namesake owner.
                if info.generic_param_ids.len() != names.len()
                    || info
                        .generic_param_ids
                        .iter()
                        .zip(names)
                        .any(|(&param, &(_, kind))| self.arena.generic_kind(param) != kind)
                {
                    info.generic_param_ids = names
                        .iter()
                        .map(|&(name, kind)| {
                            self.arena.intern_generic(GenericParamData {
                                name: source.spelling(name).into(),
                                owner_symbol_index: slot,
                                bound: None,
                                kind,
                            })
                        })
                        .collect();
                    info.generic_param_default_ids = vec![None; names.len()];
                }
            }
        }
    }

    fn rebind_module_types(&mut self) {
        self.prepare_trait_sources();
        self.prepare_elided_inputs();
        let mut pending = Vec::new();
        for input in self.modules.inputs.values() {
            let source = &self.modules.traits.files[&input.path].source;
            let import = |binding| {
                super::super::trait_graph::source_type(
                    &self.modules,
                    self,
                    &self.arena,
                    &input.path,
                    source,
                    binding,
                )
            };
            let parameter = |binding, index| {
                super::super::trait_graph::parameter(
                    &self.modules,
                    self,
                    &self.arena,
                    &input.path,
                    source,
                    binding,
                    index,
                )
            };
            for signature in &input.signatures {
                if self.by_id.contains_key(&signature.declaration) {
                    pending.push((
                        signature.declaration,
                        signature.slot,
                        signature
                            .recipes
                            .iter()
                            .map(|r| {
                                r.materialize_with_context(
                                    &self.arena,
                                    &import,
                                    &parameter,
                                    Some(self),
                                )
                            })
                            .collect(),
                    ));
                }
            }
        }
        for (id, slot, types) in pending {
            super::super::module_type_inputs::apply(
                self.type_info_by_id.entry(id).or_default(),
                slot,
                types,
            );
        }
        self.modules.traits =
            super::super::trait_graph::Graph::build(&self.modules, self, &self.arena);
        self.rebind_base_types();
    }

    pub(super) fn rebind_base_types(&mut self) {
        let bases = super::super::base_receiver::bind(&self.modules, self, &self.arena);
        for info in self.type_info_by_id.values_mut() {
            info.base_type_id = None;
        }
        let captured: FxHashSet<_> = bases.iter().map(|(id, _)| *id).collect();
        self.inherits_by_id.retain(|id, _| !captured.contains(id));
        self.inherits_args_by_pair
            .retain(|(id, _), _| !captured.contains(id));
        for (id, ty) in bases {
            self.type_info_by_id.entry(id).or_default().base_type_id = Some(ty);
            if let Some(parent) = super::super::head_decl::head_decl_id(&self.arena, ty) {
                self.inherits_by_id.insert(id, vec![parent]);
                if let Type::Apply { args, .. } = self.arena.get(ty) {
                    self.inherits_args_by_pair.insert((id, parent), args);
                }
            }
        }
    }

    fn prepare_elided_inputs(&mut self) {
        let mut sites = Vec::new();
        for input in self.modules.inputs.values() {
            let import = |binding| {
                self.modules
                    .binding(&input.path, binding, true)
                    .declaration()
                    .and_then(|id| self.symbol_by_id(id))
                    .map(|symbol| {
                        self.arena
                            .decl(&symbol.qualified_name, self.canonical_decl_id(symbol.id))
                    })
            };
            let parameter = |binding, index| {
                self.modules
                    .binding(&input.path, binding, true)
                    .declaration()
                    .and_then(|id| self.type_info_by_id.get(&self.canonical_decl_id(id)))
                    .and_then(|info| info.generic_param_ids.get(index).copied())
                    .map(|p| self.arena.generic_type(p))
            };
            for signature in &input.signatures {
                if !self.by_id.contains_key(&signature.declaration) {
                    continue;
                }
                for recipe in &signature.recipes {
                    super::super::elided_inputs::sites(
                        recipe,
                        self,
                        &self.arena,
                        &|r| {
                            r.materialize_with_context(&self.arena, &import, &parameter, Some(self))
                        },
                        &mut sites,
                        0,
                    );
                }
            }
        }
        sites.sort_unstable();
        sites.dedup();
        for (&owner, info) in &mut self.type_info_by_id {
            info.elided_input_params
                .retain(|&(byte, index, _)| sites.binary_search(&(owner, byte, index)).is_ok());
        }
        for (owner, byte, index) in sites {
            if !self
                .by_id
                .get(&owner)
                .is_some_and(|s| matches!(s.kind.as_str(), "function" | "method"))
            {
                continue;
            }
            let info = self.type_info_by_id.entry(owner).or_default();
            if info
                .elided_input_params
                .iter()
                .any(|&(site, slot, _)| (site, slot) == (byte, index))
            {
                continue;
            }
            let param = self.arena.intern_generic(GenericParamData {
                name: "'_".into(),
                owner_symbol_index: 0,
                bound: None,
                kind: crate::type_checker::core::types::GenericParamKind::Lifetime,
            });
            info.elided_input_params.push((byte, index, param));
        }
    }

    pub(super) fn load_module_bindings(
        &mut self,
        conn: &rusqlite::Connection,
    ) -> rusqlite::Result<()> {
        let mut modules = std::mem::take(&mut self.modules);
        let loaded = modules.load(conn);
        for input in modules.inputs.values() {
            self.attest_scoped_merges(&input.scoped_declarations);
        }
        self.finish_identity_passes();
        self.load_module_entries(conn, &modules);
        modules.rebuild(self);
        self.modules = modules;
        self.rebind_module_types();
        self.rebind_extension_owners();
        self.rebuild_program_views();
        loaded
    }

    fn load_module_entries(
        &mut self,
        conn: &rusqlite::Connection,
        modules: &super::super::module_graph::ModuleGraph,
    ) {
        // Persisted package-entry evidence is an ingestion input, not a symbol
        // spelling fallback. Removed/stale target modules cannot be resurrected.
        let payload: rusqlite::Result<String> = conn.query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key='module_entries_v1'",
            [],
            |r| r.get(0),
        );
        let entries = payload
            .ok()
            .and_then(|payload| serde_json::from_str::<Vec<(String, String)>>(&payload).ok());
        for (specifier, path) in entries.unwrap_or_default() {
            if modules
                .inputs
                .contains_key(&super::super::module_paths::normalize(&path))
            {
                self.module_entry.entry(specifier).or_insert(path);
            }
        }
    }
    pub(in crate::indexer::resolve::engine) fn persist_lexical_type_info(
        &self,
        conn: &rusqlite::Connection,
    ) -> rusqlite::Result<()> {
        let mut rows: Vec<_> = self.type_info_by_id.iter().collect();
        rows.sort_unstable_by_key(|(id, _)| **id);
        let payload = serde_json::to_string(&rows)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute("INSERT OR REPLACE INTO _bearwisdom_meta (key,value) VALUES ('canonical_type_info_v1',?1)", [payload])?;
        self.modules.persist(conn)?;
        let entries = serde_json::to_string(&self.module_entry.iter().collect::<Vec<_>>())
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute(
            "INSERT OR REPLACE INTO _bearwisdom_meta (key,value) VALUES ('module_entries_v1',?1)",
            [entries],
        )?;
        Ok(())
    }

    pub(super) fn load_lexical_type_info(
        &mut self,
        conn: &rusqlite::Connection,
        parsed: &FxHashSet<i64>,
    ) -> rusqlite::Result<()> {
        use rusqlite::OptionalExtension;
        let payload: Option<String> = conn
            .query_row(
                "SELECT value FROM _bearwisdom_meta WHERE key='canonical_type_info_v1'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(());
        };
        let rows: Vec<(i64, TypeInfo)> = serde_json::from_str(&payload).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;
        for (id, info) in rows {
            // Fresh parse wins; a deleted declaration cannot be resurrected by snapshot metadata.
            if !parsed.contains(&id) && self.by_id.contains_key(&id) {
                self.type_info_by_id.insert(id, info);
            }
        }
        Ok(())
    }

    pub(super) fn capture_lexical_types(&mut self, files: &[ParsedFile], ids: &SymbolIds) {
        self.prepare_elided_inputs();
        for file in files {
            let Some(graph) = &file.flow.lexical else {
                continue;
            };
            let binder = super::super::lexical_type_ids::TypeBinder {
                graph,
                path: &file.path,
                ids,
                lookup: self,
                source: Some(self),
                arena: &self.arena,
            };
            let mut pending = Vec::new();
            for (kind, recipes) in [
                (0, &graph.types.fields),
                (1, &graph.types.returns),
                (2, &graph.types.aliases),
                (3, &graph.types.receivers),
            ] {
                for (&slot, recipe) in recipes {
                    if recipe.is_bound()
                        || (kind >= 2
                            && matches!(
                                recipe,
                                crate::indexer::lexical::type_syntax::TypeExpr::Unknown
                            ))
                    {
                        if let Some(id) = ids.row_id(&file.path, slot) {
                            pending.push((id, kind, binder.materialize(recipe)));
                        }
                    }
                }
            }
            let parameters: Vec<_> = graph
                .types
                .parameters
                .iter()
                .filter_map(|(&slot, recipes)| {
                    Some((
                        ids.row_id(&file.path, slot)?,
                        recipes.iter().map(|r| binder.materialize(r)).collect(),
                    ))
                })
                .collect();
            for (id, params) in parameters {
                self.type_info_by_id
                    .entry(id)
                    .or_default()
                    .parameter_type_ids = Some(params);
            }
            for (id, kind, ty) in pending {
                let info = self.type_info_by_id.entry(id).or_default();
                match kind {
                    0 => info.field_type_id = Some(ty),
                    1 => info.return_type_id = Some(ty),
                    3 => info.receiver_type_id = Some(ty),
                    _ => {
                        info.lexical_alias = Some(
                            super::super::contract::generic_return::GenericReturn::bound(
                                info.generic_param_ids.clone(),
                                info.generic_param_default_ids.clone(),
                                ty,
                            ),
                        )
                    }
                }
            }
        }
        self.rebind_module_types();
        self.rebind_extension_owners();
        self.rebuild_program_views();
    }
}

#[cfg(test)]
#[path = "compilation_type_bindings_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "compilation_trait_inputs_tests.rs"]
mod trait_tests;
#[path = "compilation_trait_bindings.rs"]
mod traits;
