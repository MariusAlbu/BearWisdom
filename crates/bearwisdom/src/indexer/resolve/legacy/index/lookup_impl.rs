// =============================================================================
// indexer/resolve/engine/index/lookup_impl.rs — SymbolLookup trait impl
//
// All 30 `SymbolLookup` methods for `SymbolIndex`. Pure read-side trait
// implementation — every method reads a field of SymbolIndex (or the
// thread-local `LOCAL_TYPE_CACHE` for flow-typing) and returns. No
// construction or mutation logic; that's in `build.rs` / `augment.rs`.
// =============================================================================

use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::AliasTarget;

use super::{strip_generic_args, SymbolIndex, CURRENT_FILE_MISSES, LOCAL_TYPE_CACHE};
use crate::indexer::resolve::legacy::{SymbolInfo, SymbolLookup, SymbolSet};

impl SymbolIndex {
    /// Merge an eager-internal slice with materialized-external hits into one
    /// `SymbolSet`. The common internal-only case (no materialized hits)
    /// borrows the eager slice with zero allocation; otherwise the stores are
    /// concatenated eager-first, so an internal symbol precedes an external one
    /// on a simple-name tie (preserving "local beats imported").
    fn span_eager_materialized<'a>(
        &'a self,
        eager: &'a [SymbolInfo],
        materialized: Vec<&'a SymbolInfo>,
    ) -> SymbolSet<'a> {
        if materialized.is_empty() {
            SymbolSet::Borrowed(eager)
        } else {
            let mut all: Vec<&SymbolInfo> = eager.iter().collect();
            all.extend(materialized);
            SymbolSet::Owned(all)
        }
    }
}

impl SymbolLookup for SymbolIndex {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        let eager = self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[]);
        let mut mat = self.materialized.by_name(name);
        if eager.is_empty() && mat.is_empty() {
            // Both stores miss → pull the external file(s) that define this name
            // and re-query. Idempotent: an already-materialized file is a no-op.
            self.materialize_by_name(name);
            mat = self.materialized.by_name(name);
        }
        self.span_eager_materialized(eager, mat)
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        // Eager-internal wins on a qname tie, preserving "local beats imported".
        if let Some(s) = self.by_qname.get(qname) {
            return Some(s);
        }
        if let Some(s) = self.materialized.by_qualified_name(qname) {
            return Some(s);
        }
        // Miss both → materialize by the leaf name, then re-query by qname.
        self.materialize_by_name(qname.rsplit('.').next().unwrap_or(qname));
        self.materialized.by_qualified_name(qname)
    }

    fn all_by_qualified_name(&self, qname: &str) -> SymbolSet<'_> {
        // Fast path: the common case is one symbol per qname. Return the
        // single-winner slice backed by `by_qname` without allocating.
        // When a duplicate exists we fall through to the combined vec built
        // at construction time — but we can't splice the by_qname winner
        // onto the duplicates slice at read time without allocating, so at
        // build time we stash the full set (winner + duplicates) under the
        // qname in `qname_duplicates` whenever it grows past one entry.
        if let Some(all) = self.qname_duplicates.get(qname) {
            return SymbolSet::Borrowed(all.as_slice());
        }
        match self.by_qname.get(qname) {
            Some(s) => SymbolSet::Borrowed(std::slice::from_ref(s)),
            None => SymbolSet::empty(),
        }
    }

    fn members_of(&self, parent_qname: &str) -> SymbolSet<'_> {
        let eager = self
            .members_by_parent
            .get(parent_qname)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let mut mat = self.materialized.members_of(parent_qname);
        if eager.is_empty() && mat.is_empty() {
            // The parent type's file may not be pulled yet — materialize by its
            // leaf name so its members land under this parent qname.
            self.materialize_by_name(parent_qname.rsplit('.').next().unwrap_or(parent_qname));
            mat = self.materialized.members_of(parent_qname);
        }
        self.span_eager_materialized(eager, mat)
    }

    fn types_by_name(&self, name: &str) -> SymbolSet<'_> {
        let eager = self
            .types_by_name
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let mut mat = self.materialized.types_by_name(name);
        // Trigger materialization only when the name is unknown internally at
        // ALL (`by_name` empty) — an `is-this-a-type?` probe fires for many bare
        // names, and pulling for an internal NON-type whose name merely matches
        // an external symbol is the over-pull that flips other edges to
        // external_refs. Gating on `by_name` keeps the pull to genuinely-external
        // type names (Result, PartialEq, Debug, …).
        if eager.is_empty() && mat.is_empty() && !self.by_name.contains_key(name) {
            self.materialize_by_name(name);
            mat = self.materialized.types_by_name(name);
        }
        self.span_eager_materialized(eager, mat)
    }

    /// O(log N) prefix search via BTreeMap::range — no extra Vec needed.
    fn in_namespace(&self, namespace: &str) -> Vec<&SymbolInfo> {
        let prefix = format!("{namespace}.");
        // Range lower bound is the owned prefix String so K: Borrow<K> = String: Borrow<String>
        // is satisfied; take_while stops as soon as keys no longer share the prefix.
        self.by_qname
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| v)
            .collect()
    }

    fn has_in_namespace(&self, namespace: &str) -> bool {
        // The "is this an internal namespace?" probe used by every language's
        // `infer_external` chain to gate the structural fallback — see
        // python/resolve.rs::module_is_external, java/resolve.rs, etc. External
        // symbols must NOT poison this check: indexing `Lib/difflib.py`
        // produces qnames under `difflib.*`, which would otherwise mark
        // `import difflib` as "internal" and short-circuit the external
        // classification, leaving every stdlib import the static stdlib list
        // misses stuck in unresolved_refs. Restrict the namespace probe to
        // symbols whose `file_path` does not start with the `ext:` virtual
        // prefix — only project-internal symbols qualify a namespace as local.
        let prefix = format!("{namespace}.");
        self.by_qname
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .any(|(_, info)| !info.file_path.starts_with("ext:"))
    }

    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        // Exact match
        if let Some(syms) = self.by_file.get(file_path) {
            return SymbolSet::Borrowed(syms.as_slice());
        }
        // Module specifier → resolved file path
        if let Some(resolved) = self.module_to_file.get(file_path) {
            if let Some(syms) = self.by_file.get(resolved) {
                return SymbolSet::Borrowed(syms.as_slice());
            }
        }
        SymbolSet::Borrowed(&self.empty)
    }

    fn in_module_from(&self, source_file: &str, spec: &str) -> SymbolSet<'_> {
        // Per-source resolution wins. Relative specifiers require this, and
        // some ecosystems also have bare specifiers whose nearest-file meaning
        // depends on the importing directory.
        if let Some(resolved) = self
            .module_to_file_per_source
            .get(&(source_file.to_string(), spec.to_string()))
        {
            if let Some(syms) = self.by_file.get(resolved) {
                return SymbolSet::Borrowed(syms.as_slice());
            }
        }
        if spec.starts_with('.') {
            // Backward-compat: if the spec literally matches an indexed
            // file path (test fixtures often use the spec as the path),
            // surface it. Real-world relative specs like `./utils` won't
            // collide with indexed paths so this is harmless.
        }
        self.in_file(spec)
    }

    fn resolve_module_from(&self, source_file: &str, spec: &str) -> Option<&str> {
        if let Some(resolved) = self
            .module_to_file_per_source
            .get(&(source_file.to_string(), spec.to_string()))
        {
            return Some(resolved.as_str());
        }
        self.module_to_file.get(spec).map(|s| s.as_str())
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.type_info
            .get(property_qname)
            .and_then(|ti| ti.field_type.as_deref())
            .or_else(|| self.materialized.field_type_name(property_qname))
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.type_info
            .get(method_qname)
            .and_then(|ti| ti.return_type.as_deref())
            .or_else(|| self.materialized.return_type_name(method_qname))
    }

    fn field_type_args(&self, property_qname: &str) -> Option<&[String]> {
        self.type_info
            .get(property_qname)
            .and_then(|ti| {
                if ti.type_args.is_empty() {
                    None
                } else {
                    Some(ti.type_args.as_slice())
                }
            })
            .or_else(|| self.materialized.field_type_args(property_qname))
    }

    fn return_type_args(&self, method_qname: &str) -> Option<&[String]> {
        self.type_info
            .get(method_qname)
            .and_then(|ti| {
                if ti.return_type_args.is_empty() {
                    None
                } else {
                    Some(ti.return_type_args.as_slice())
                }
            })
            .or_else(|| self.materialized.return_type_args(method_qname))
    }

    fn generic_params(&self, type_name: &str) -> Option<&[String]> {
        self.type_info
            .get(type_name)
            .and_then(|ti| {
                if ti.generic_params.is_empty() {
                    None
                } else {
                    Some(ti.generic_params.as_slice())
                }
            })
            .or_else(|| self.materialized.generic_params(type_name))
    }

    fn field_type_id(&self, property_qname: &str) -> Option<TypeId> {
        self.type_info
            .get(property_qname)
            .and_then(|ti| ti.field_type_id)
    }

    fn return_type_id(&self, method_qname: &str) -> Option<TypeId> {
        self.type_info
            .get(method_qname)
            .and_then(|ti| ti.return_type_id)
    }

    fn field_type_arg_ids(&self, property_qname: &str) -> Option<&[TypeId]> {
        self.type_info.get(property_qname).and_then(|ti| {
            if ti.type_arg_ids.is_empty() {
                None
            } else {
                Some(ti.type_arg_ids.as_slice())
            }
        })
    }

    fn generic_param_type_ids(&self, type_name: &str) -> Option<&[TypeId]> {
        self.type_info.get(type_name).and_then(|ti| {
            if ti.generic_param_type_ids.is_empty() {
                None
            } else {
                Some(ti.generic_param_type_ids.as_slice())
            }
        })
    }

    fn type_arena(&self) -> Option<&TypeArena> {
        Some(&self.type_arena)
    }

    fn alias_target(&self, name: &str) -> Option<&AliasTarget> {
        self.alias_target.get(name)
    }

    fn reexports_from(&self, file_path: &str) -> &[(String, String)] {
        // Exact match
        if let Some(v) = self.reexport_map.get(file_path) {
            return v.as_slice();
        }
        // Module specifier → resolved file path
        if let Some(resolved) = self.module_to_file.get(file_path) {
            if let Some(v) = self.reexport_map.get(resolved) {
                return v.as_slice();
            }
        }
        &self.empty_reexports
    }

    fn is_external_name(&self, name: &str, language: &str) -> bool {
        // Language primitive check. Name is not in the project index at all
        // and is not a known local name — we intentionally do NOT call
        // by_name here, that's for the resolution path.
        if let Some(primitives) = self.primitives_by_language.get(language) {
            if primitives.contains(name) {
                return true;
            }
        }
        false
    }

    fn is_external_file(&self, path: &str) -> bool {
        path.starts_with("ext:") || self.external_paths.contains(path)
    }

    fn is_ambient_global_method(&self, name: &str) -> bool {
        self.ambient_global_method_names.contains(name)
    }

    fn is_ambient_path(&self, path: &str) -> bool {
        SymbolIndex::is_ambient_path(self, path)
    }

    fn resolve_external_reexport(
        &self,
        target_name: &str,
        chain_prefix: &str,
        module_path: &str,
    ) -> Option<i64> {
        SymbolIndex::resolve_via_external_reexport(
            self,
            target_name,
            chain_prefix,
            module_path,
            &[],
        )
    }

    fn symbols_in_package(&self, package_id: i64) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_package
                .get(&package_id)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }

    fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        if let Some(&id) = self.workspace_pkg_by_declared_name.get(specifier) {
            return Some(id);
        }
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if let Some(&id) = self.workspace_pkg_by_declared_name.get(path) {
                return Some(id);
            }
        }
        None
    }

    fn is_workspace_declared_name(&self, name: &str) -> bool {
        self.workspace_pkg_by_declared_name.contains_key(name)
    }

    fn resolve_path_alias(&self, package_id: Option<i64>, specifier: &str) -> Option<String> {
        let paths = package_id
            .and_then(|id| self.path_aliases_by_pkg.get(&id))
            .map(|v| v.as_slice())
            .unwrap_or(self.path_aliases_union.as_slice());
        if paths.is_empty() {
            return None;
        }
        // Pick the longest matching alias so nested prefixes win.
        let mut best: Option<&(String, String)> = None;
        for entry in paths {
            let (alias, _) = entry;
            if specifier.starts_with(alias.as_str())
                && best.map_or(true, |(b, _)| alias.len() > b.len())
            {
                best = Some(entry);
            }
        }
        let (alias, target) = best?;
        let remainder = &specifier[alias.len()..];
        Some(format!("{target}{remainder}"))
    }

    fn parent_class_qname(&self, class_qname: &str) -> Option<&str> {
        if let Some(child) = self.by_qname.get(class_qname) {
            if let Some(parent_id) = self.inherits_by_id.get(&child.id) {
                if let Some(parent) = self.by_id.get(parent_id) {
                    return Some(parent.qualified_name.as_str());
                }
            }
        }
        self.materialized.parent_class_qname(class_qname)
    }

    fn enclosing_type_qname(&self, source_qname: &str) -> Option<&str> {
        let start = self.by_qname.get(source_qname)?;
        // Walk the containment edge upward (excluding self), returning the
        // first ancestor whose kind is a type. The bounded hop count guards a
        // malformed (cyclic) containing_id chain.
        let mut cur = self.containing_id.get(&start.id).copied();
        for _ in 0..256 {
            let pid = cur?;
            let info = self.by_id.get(&pid)?;
            // A type-defining ancestor only (Roslyn's ContainingType): namespaces,
            // type aliases, etc. are NOT enclosing types. This is the kind set the
            // resolver's enclosing-type lookup has always used.
            if matches!(
                info.kind.as_str(),
                "class" | "struct" | "interface" | "trait" | "enum"
            ) {
                return Some(info.qualified_name.as_str());
            }
            cur = self.containing_id.get(&pid).copied();
        }
        None
    }

    fn enclosing_namespace_qname(&self, source_qname: &str) -> Option<&str> {
        let start = self.by_qname.get(source_qname)?;
        // Nearest namespace/module ancestor via the containment edge. When the
        // chain tops out without one, the package is hoisted into scope_path
        // (Java-style) rather than modeled as a parent symbol — fall back to
        // the topmost reached symbol's scope_path, which is that package.
        let mut topmost = start;
        let mut cur = self.containing_id.get(&start.id).copied();
        for _ in 0..256 {
            let Some(pid) = cur else { break };
            let Some(info) = self.by_id.get(&pid) else {
                break;
            };
            if matches!(info.kind.as_str(), "namespace" | "module") {
                return Some(info.qualified_name.as_str());
            }
            topmost = info;
            cur = self.containing_id.get(&pid).copied();
        }
        topmost.scope_path.as_deref()
    }

    fn selector_qname(&self, raw_selector: &str) -> Option<&str> {
        self.angular_selectors.get(raw_selector).map(|s| s.as_str())
    }

    fn record_chain_miss(&self, target_name: &str) {
        // A chain bail-out marks the file currently being resolved on this
        // worker for the next frontier pass. The source file is attributed at
        // drain time (`take_file_misses`), so the miss itself carries only the
        // unresolved segment name — generics stripped (`Promise<User>` →
        // `Promise`) so it matches bare index entries when the frontier is
        // later narrowed against newly-inferred returns.
        CURRENT_FILE_MISSES.with(|c| c.borrow_mut().push(strip_generic_args(target_name)));
    }

    fn local_type(&self, name: &str) -> Option<String> {
        LOCAL_TYPE_CACHE
            .with(|c| c.borrow().last().and_then(|t| t.lookup(name).map(|s| s.to_string())))
    }

    fn local_type_union(&self, name: &str) -> Option<Vec<String>> {
        LOCAL_TYPE_CACHE.with(|c| c.borrow().last().and_then(|t| t.lookup_union(name)))
    }

    fn local_discriminant(&self, name: &str) -> Option<(String, String, bool)> {
        LOCAL_TYPE_CACHE.with(|c| {
            c.borrow().last().and_then(|t| {
                t.discriminant(name)
                    .map(|(p, l, n)| (p.to_string(), l.to_string(), n))
            })
        })
    }

    fn install_local_cache(
        &self,
        narrowings: Vec<crate::types::Narrowing>,
        discriminants: Vec<crate::types::DiscriminantNarrowing>,
        cfg: crate::indexer::flow_cfg::FileCfg,
    ) {
        LOCAL_TYPE_CACHE.with(|c| {
            let mut stack = c.borrow_mut();
            let cache = stack.last_mut().expect("local-type stack has a base scope");
            cache.forward.clear();
            cache.narrowings = narrowings;
            cache.discriminants = discriminants;
            cache.cfg = cfg;
            cache.cursor = 0;
        });
    }

    fn set_cursor(&self, byte: u32) {
        LOCAL_TYPE_CACHE.with(|c| {
            if let Some(cache) = c.borrow_mut().last_mut() {
                cache.cursor = byte;
            }
        });
    }

    fn record_local_type(&self, name: String, type_name: String) {
        LOCAL_TYPE_CACHE.with(|c| {
            if let Some(cache) = c.borrow_mut().last_mut() {
                cache.forward.insert(name, type_name);
            }
        });
    }

    fn clear_local_cache(&self) {
        LOCAL_TYPE_CACHE.with(|c| {
            let mut stack = c.borrow_mut();
            let cache = stack.last_mut().expect("local-type stack has a base scope");
            cache.forward.clear();
            cache.narrowings.clear();
            cache.discriminants.clear();
            cache.cfg = crate::indexer::flow_cfg::FileCfg::default();
            cache.cursor = 0;
        });
    }
}

#[cfg(test)]
#[path = "lookup_impl_tests.rs"]
mod tests;
