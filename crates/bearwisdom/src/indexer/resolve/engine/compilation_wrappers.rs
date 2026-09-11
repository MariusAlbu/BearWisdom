//! Wrapper-return inference. Declaration-ID writes remain separate from the
//! legacy display/name-bucket compatibility store.
use super::*;

impl Compilation {
    /// Infer a function's return type from a `return <call>` whose value IS the
    /// call's return: `function usePost() { return useQuery(...) }` makes
    /// `usePost`'s return `useQuery`'s. `flow.flow_return_lhs` links the returned
    /// call's ref to its enclosing function; this resolves that call's return in
    /// the function's own import scope (so the right package's overload is read)
    /// and fills the function's return slot. The call-return mirror of
    /// `infer_bare_identifier_returns` (a returned IDENTIFIER), folded through the
    /// same cross-owner agreement gate. Only fills a genuine gap, and only for a
    /// DIRECT call return — a chained `return a.b()` is left to the chain walker.
    pub(crate) fn infer_call_wrapper_returns(&mut self, parsed: &[ParsedFile], ids: &SymbolIds) {
        let mut by_id: FxHashMap<i64, Vec<Option<TypeId>>> = FxHashMap::default();
        // Keep semantic ids through the whole wrapper pass. A wrapper return is
        // resolved from another declaration and therefore must never be rendered
        // and reparsed according to the wrapper's source language.
        let mut candidates: Vec<(String, TypeId)> = Vec::new();
        for pf in parsed.iter().filter(|p| !p.path.starts_with("ext:")) {
            if pf.flow.flow_return_lhs.is_empty() {
                continue;
            }
            let imports: Vec<ImportEntry> = pf
                .refs
                .iter()
                .filter(|r| r.is_import_binding)
                .filter_map(|r| {
                    let module = r.module.clone()?;
                    Some(ImportEntry {
                        imported_name: r.target_name.clone(),
                        module_path: Some(module),
                        alias: None,
                        is_wildcard: r.target_name == "*",
                        binding_kind: r
                            .chain
                            .as_ref()
                            .and_then(|chain| chain.segments.first())
                            .map(|segment| segment.kind),
                    })
                })
                .collect();
            let file_ctx = FileContext {
                file_path: pf.path.clone(),
                language: pf.language.clone(),
                imports,
                file_namespace: None,
            };
            for (ref_idx, fn_idx) in &pf.flow.flow_return_lhs {
                let Some(fn_sym) = pf.symbols.get(*fn_idx) else {
                    continue;
                };
                // A declared/extractor return annotation wins. An already-INFERRED
                // slot does NOT short-circuit: a factory's object-literal-builder
                // return (`{…}$Ret`) is authoritative and overrides a slot
                // mis-inferred from a param (`Record`) / body (`Promise`) — decided
                // in the grouping pass below.
                if fn_sym.return_type.is_some()
                    || pf
                        .flow
                        .lexical
                        .as_ref()
                        .is_some_and(|graph| graph.types.returns.contains_key(fn_idx))
                {
                    continue;
                }
                let Some(call_ref) = pf.refs.get(*ref_idx) else {
                    continue;
                };
                // Direct call only; a chained `return a.b()` (multi-segment) is
                // the chain walker's job, not this single-callee inference.
                if call_ref
                    .chain
                    .as_ref()
                    .map(|c| c.segments.len())
                    .unwrap_or(1)
                    > 1
                {
                    continue;
                }
                let ret = lexical_return(self, pf, ids, call_ref).unwrap_or_else(|| {
                    super::super::chain::callee_return_type_in_scope(
                        self,
                        &self.arena,
                        &file_ctx,
                        &call_ref.target_name,
                        &fn_sym.qualified_name,
                    )
                });
                if let Some(owner) = ids.row_id(&pf.path, *fn_idx) {
                    by_id.entry(owner).or_default().push(ret);
                }
                let Some(ret_id) = ret else {
                    continue;
                };
                if matches!(self.arena.get(ret_id), Type::Unknown) {
                    continue;
                }
                candidates.push((fn_sym.qualified_name.clone(), ret_id));
            }
        }
        // Group candidates by function. Object-literal-builder returns (`{…}$Ret`)
        // are authoritative: a factory returning one or more local builders HAS
        // that (union of) object shape(s) as its return, overriding any stored
        // return mis-inferred from a param/body. A non-builder single return keeps
        // the agreement-gated, slot-respecting behaviour.
        let mut by_fn: FxHashMap<String, Vec<TypeId>> = FxHashMap::default();
        for (qname, ty_id) in candidates {
            by_fn.entry(qname).or_default().push(ty_id);
        }
        for (qname, variants) in by_fn {
            // Distinct semantic return types, first occurrence kept.
            let mut distinct: Vec<TypeId> = Vec::new();
            for v in variants {
                if !distinct.contains(&v) {
                    distinct.push(v);
                }
            }
            let mut ret_branches: Vec<String> = distinct
                .iter()
                .filter_map(|&id| match self.arena.get(id) {
                    Type::Class(name) if name.ends_with("$Ret") => Some(name.to_string()),
                    _ => None,
                })
                .collect();
            if !ret_branches.is_empty() {
                let (ret_str, ret_id) = if ret_branches.len() == 1 {
                    let n = ret_branches.pop().unwrap();
                    let id = self.arena.class(&n);
                    (n, id)
                } else {
                    ret_branches.sort(); // member-on-all-branches is order-free
                    ret_branches.dedup();
                    let union_name = format!("{qname}$Ret");
                    self.alias_target.insert(
                        union_name.clone(),
                        AliasTargetIds::Union(
                            ret_branches
                                .iter()
                                .map(|name| self.arena.class(name))
                                .collect(),
                        ),
                    );
                    let id = self.arena.class(&union_name);
                    (union_name, id)
                };
                self.set_return_both(&qname, ret_str, ret_id);
                self.mirror_ret_interface_member(&qname, ret_id);
                continue;
            }
            // No object-literal builder branch: keep a single agreed non-synthetic
            // return, not overriding an existing slot (the wrapper-hook case).
            if self
                .type_info
                .get(&qname)
                .and_then(|ti| ti.return_type_id)
                .is_some()
            {
                continue;
            }
            if distinct.len() == 1 {
                let rid = distinct[0];
                let ti = self.type_info.entry(qname.clone()).or_default();
                ti.return_type_id = Some(rid);
                self.mirror_ret_interface_member(&qname, rid);
            }
        }
        // ID owners never borrow an agreed name-bucket return from another owner.
        for (owner, candidates) in by_id {
            self.type_info_by_id
                .entry(owner)
                .or_default()
                .return_type_id = Some(agreed_return(&self.arena, &candidates));
        }
        super::super::contract::generic_return::capture_all(&self.arena, &mut self.type_info_by_id);
    }
}

// None: unmigrated root; Some(None): known root whose type is missing.
fn lexical_return(
    tree: &Compilation,
    file: &ParsedFile,
    ids: &SymbolIds,
    call: &crate::types::ExtractedRef,
) -> Option<Option<TypeId>> {
    let graph = file.flow.lexical.as_ref()?;
    let binding = graph.references.get(&call.byte_offset)?;
    if graph.module.imports.contains_key(binding) {
        if let Some(ty) = super::super::lexical_value::imported_overload_type(
            tree,
            &tree.arena,
            &file.path,
            *binding,
        ) {
            return Some(super::super::lexical_value::callable_return(
                &tree.arena,
                ty,
            ));
        }
    }
    let declaration = if graph.module.imports.contains_key(binding) {
        tree.bound_import(&file.path, *binding, false)
    } else {
        graph
            .symbol_slots
            .get(binding)
            .copied()
            .flatten()
            .and_then(|slot| ids.row_id(&file.path, slot))
    };
    Some(declaration.and_then(|id| tree.return_type_id_of(id)))
}

fn agreed_return(arena: &TypeArena, candidates: &[Option<TypeId>]) -> TypeId {
    match candidates.first().copied().flatten() {
        Some(first) if candidates.iter().all(|candidate| *candidate == Some(first)) => first,
        _ => arena.intern(Type::Unknown),
    }
}

#[cfg(test)]
#[path = "compilation_wrappers_tests.rs"]
mod tests;
