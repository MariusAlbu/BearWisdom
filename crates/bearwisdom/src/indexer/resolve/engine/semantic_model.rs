// =============================================================================
// engine/semantic_model — the new engine's entry: solve one code reference
//
// The `SemanticModel` owns the production rule ladder and resolves a ref against
// it. Mirrors the old engine's `resolve_generic` entry: build the profile-driven
// kind predicate, assemble a `BinderContext`, run the rules. Chain-bearing refs
// (member access) are walked by the reused `ChainWalker` structure — wired in a
// later step; this entry covers the chain-less ladder the lifted rules
// implement.
// =============================================================================

use std::str::FromStr;

use crate::indexer::resolve::engine::cause::{Cause, CauseKind};
use crate::indexer::resolve::engine::contract::{
    FileContext, RefContext, SymbolInfo, SymbolLookup,
};
use crate::type_checker::profile::language_profile::{
    KindCompatibility, KindTable, LanguageProfile,
};
use crate::types::{EdgeKind, SymbolKind};

use super::{BindOutcome, Binder, BinderContext};

/// Outcome of solving one ref, chain-bearing or chain-less.
pub enum SolveOutcome {
    /// A rule or the chain walk bound the ref.
    Resolved(SymbolInfo),
    /// Nothing bound it — an honest miss, carrying the first-uncaptured-type
    /// cause when a death site could attribute one.
    Unresolved(Option<Cause>),
    /// The rule ladder declined the ref as a known non-project construct
    /// (`LanguageProfile::builtin_skip`) rather than a missing project symbol.
    Drained,
}

/// The rule-based code-reference solver. Holds the ordered rule set and applies
/// it to one ref at a time.
pub struct SemanticModel {
    engine: Binder,
    /// Module-evidence subset for a declined member chain that carries an
    /// extractor-set `module` — every rung in it is scoped by that module, so
    /// the fall-through can never bind an unrelated same-named sibling.
    module_engine: Binder,
}

impl SemanticModel {
    /// Build a solver over the production rule ladder.
    pub fn production() -> Self {
        Self {
            engine: Binder::production(),
            module_engine: Binder::module_evidence(),
        }
    }

    /// Build a solver over an explicit rule set (tests inject a subset).
    pub fn new(engine: Binder) -> Self {
        Self {
            engine,
            module_engine: Binder::module_evidence(),
        }
    }

    /// Solve one ref. A chain-bearing ref (member access) walks the Symbol tree;
    /// a chain-less ref runs the rule ladder. There is no old-engine fallback —
    /// a ref no rule and no chain hop resolves is honestly unresolved.
    pub fn get_symbol_info(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> SolveOutcome {
        let reference = ref_ctx.extracted_ref;
        if matches!(reference.kind, EdgeKind::Calls | EdgeKind::Instantiates)
            && reference
                .chain
                .as_ref()
                .is_none_or(|c| c.segments.len() < 2)
        {
            if let Some(local) = lookup.local_reference(reference.byte_offset) {
                let Some(target_symbol_id) = local.declaration else {
                    // Known binding, missing/ambiguous persisted row: no global fallback.
                    return SolveOutcome::Unresolved(None);
                };
                let resolved_yield_type = lookup.type_arena().and_then(|arena| {
                    super::lexical_value::yield_type(
                        &local,
                        lookup,
                        arena,
                        reference.kind == EdgeKind::Instantiates,
                    )
                });
                return SolveOutcome::Resolved(SymbolInfo {
                    target_symbol_id,
                    confidence: super::contract::RESOLVED_CONFIDENCE,
                    strategy: "lexical_binding",
                    resolved_yield_type,
                    flow_emit: None,
                });
            }
        }
        // The walk's own diagnosis of a declined chain. When a namespace or
        // wildcard-import root sends the chain through the module-scoped ladder
        // below and that ladder also comes up empty, this outranks a bare-name
        // classification of the last segment: the root that never linked or
        // the member the receiver lacks is the cause, not the leaf's name.
        let mut chain_cause: Option<Cause> = None;
        if let Some(chain) = ref_ctx.extracted_ref.chain.as_ref() {
            match super::chain::bind_member_access(ref_ctx, file_ctx, lookup, profile) {
                Ok(res) => return SolveOutcome::Resolved(res),
                Err(cause) => {
                    chain_cause = cause;
                    if chain
                        .segments
                        .first()
                        .is_some_and(|s| s.kind == crate::types::SegmentKind::BaseRef)
                    {
                        return SolveOutcome::Unresolved(cause);
                    }
                    if chain.segments.len() > 1
                        && lookup.local_reference(reference.byte_offset).is_some()
                    {
                        return SolveOutcome::Unresolved(cause);
                    }
                    // A multi-segment chain the walk declined is normally a genuine
                    // miss: a same-named sibling must not hijack `a.b.c`. The only
                    // exception is a root whose module is proven at THIS source
                    // site: an imported/ambient/same-package namespace, a wildcard
                    // import alias, or a ref whose own `module` is its qualifier.
                    // Re-run only the module-evidence subset with those source-
                    // addressed module paths; never the full bare-name ladder.
                    if chain.segments.len() > 1 {
                        let mut modules = namespace_root_modules(
                            chain,
                            file_ctx,
                            ref_ctx.file_package_id,
                            lookup,
                        );
                        for module in wildcard_root_modules(chain, file_ctx) {
                            push_unique_module(&mut modules, &module);
                        }
                        if module_is_chain_qualifier(ref_ctx.extracted_ref, chain, profile) {
                            if let Some(module) = ref_ctx.extracted_ref.module.as_deref() {
                                push_unique_module(&mut modules, module);
                            }
                        }

                        // A declined walk without a cause of its own anchored the
                        // root (a failed anchor always carries one) and died on a
                        // later hop silently — record that as the chain's cause.
                        let cause = cause.or(Some(Cause::new(None, CauseKind::ChainDeclined)));
                        if !modules.is_empty() {
                            return match self.resolve_module_scoped_at(
                                ref_ctx, file_ctx, lookup, profile, &modules,
                            ) {
                                BindOutcome::Resolved(res, _rule) => SolveOutcome::Resolved(res),
                                BindOutcome::Drained => SolveOutcome::Drained,
                                BindOutcome::Unresolved => SolveOutcome::Unresolved(cause),
                            };
                        }
                        return SolveOutcome::Unresolved(cause);
                    }
                }
            }
        }
        match self.resolve_chain_less(ref_ctx, file_ctx, lookup, profile) {
            BindOutcome::Resolved(res, _rule) => SolveOutcome::Resolved(res),
            BindOutcome::Unresolved => SolveOutcome::Unresolved(chain_cause.or_else(|| {
                Some(super::unbound_cause::classify_unbound_root(
                    &ref_ctx.extracted_ref.target_name,
                    &ref_ctx.scope_chain,
                    file_ctx,
                    lookup,
                    ref_ctx.file_package_id,
                ))
            })),
            BindOutcome::Drained => SolveOutcome::Drained,
        }
    }

    /// Solve one chain-less ref through the rule ladder. Builds the profile kind
    /// predicate, assembles a `BinderContext`, and runs the rules. See
    /// [`BindOutcome`].
    pub fn resolve_chain_less(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> BindOutcome {
        let table = profile.kind_compatible_table;
        let kind = move |edge: EdgeKind, sym_kind: &str| kind_ok_table(table, edge, sym_kind);
        let ctx = BinderContext {
            file_ctx,
            ref_ctx,
            lookup,
            kind: &kind,
            profile,
        };
        self.engine.bind(&ctx)
    }

    /// Solve a declined module-tagged member chain through the module-evidence
    /// rung subset. Same context assembly as `resolve_chain_less`; only the
    /// rule set differs.
    fn resolve_module_scoped(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> BindOutcome {
        let table = profile.kind_compatible_table;
        let kind = move |edge: EdgeKind, sym_kind: &str| kind_ok_table(table, edge, sym_kind);
        let ctx = BinderContext {
            file_ctx,
            ref_ctx,
            lookup,
            kind: &kind,
            profile,
        };
        self.module_engine.bind(&ctx)
    }

    /// Run a declined namespace/wildcard chain only through modules the source
    /// site proves. Each attempt gets a synthetic `ref.module`, so the existing
    /// module-evidence rules remain the sole binding surface.
    fn resolve_module_scoped_at(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
        modules: &[String],
    ) -> BindOutcome {
        for module in modules {
            let mut scoped_ref = ref_ctx.extracted_ref.clone();
            scoped_ref.module = Some(module.clone());
            let scoped_ctx = RefContext {
                extracted_ref: &scoped_ref,
                source_symbol: ref_ctx.source_symbol,
                scope_chain: ref_ctx.scope_chain.clone(),
                file_package_id: ref_ctx.file_package_id,
                source_symbol_id: ref_ctx.source_symbol_id,
            };
            match self.resolve_module_scoped(&scoped_ctx, file_ctx, lookup, profile) {
                BindOutcome::Resolved(res, rule) => return BindOutcome::Resolved(res, rule),
                BindOutcome::Drained => return BindOutcome::Drained,
                BindOutcome::Unresolved => {}
            }
        }
        BindOutcome::Unresolved
    }
}

/// `true` when the ref's extractor-set `module` is exactly the chain's own
/// qualifier path — every segment but the target, joined by the profile's
/// separator (or the universal `.`). That shape means the target is a DIRECT
/// member of the module (`serde_json::from_value` → module `serde_json`,
/// chain `[serde_json, from_value]`), so the module-evidence rungs can bind
/// it. A module tag naming where the chain's root was imported from joins to
/// a different string and is rejected.
fn module_is_chain_qualifier(
    r: &crate::types::ExtractedRef,
    chain: &crate::types::MemberChain,
    profile: &LanguageProfile,
) -> bool {
    let Some(module) = r.module.as_deref() else {
        return false;
    };
    let quals = &chain.segments[..chain.segments.len() - 1];
    let mut for_sep = |sep: &str| {
        let joined = quals
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(sep);
        joined == module
    };
    for_sep(profile.qname_separator) || for_sep(".")
}

/// Source-addressed module paths a namespace root may use after its value walk
/// declines. A same-file value wins before any namespace candidate; an imported
/// root must tie its namespace declaration to that import's module/package/path.
/// A same-named namespace elsewhere in the index supplies no module evidence.
fn namespace_root_modules(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
) -> Vec<String> {
    let Some(root) = chain.segments.first() else {
        return Vec::new();
    };
    let candidates = lookup.by_name(&root.name);
    if candidates.iter().any(|s| {
        s.file_path.as_ref() == file_ctx.file_path
            && crate::indexer::resolve::engine::kinds::is_value_kind(&s.kind)
    }) {
        return Vec::new();
    }

    let is_namespace = |kind: &str| matches!(kind, "namespace" | "module");
    let mut modules = Vec::new();
    for candidate in candidates.iter().filter(|s| is_namespace(&s.kind)) {
        let imported = file_ctx.imports.iter().any(|import| {
            !import.is_wildcard
                && import.bound_name() == root.name.as_str()
                && namespace_matches_import(candidate, import, file_ctx, lookup)
        });
        let same_package =
            file_package_id.is_some_and(|package_id| candidate.package_id == Some(package_id));
        if imported || same_package {
            push_unique_module(&mut modules, &candidate.qualified_name);
            if imported {
                for import in &file_ctx.imports {
                    if !import.is_wildcard
                        && import.bound_name() == root.name.as_str()
                        && namespace_matches_import(candidate, import, file_ctx, lookup)
                    {
                        if let Some(module) = import.module_path.as_deref() {
                            push_unique_module(&mut modules, module);
                        }
                    }
                }
            }
        }
    }
    // Ambient declarations are source-visible even when an index only keeps
    // them in its ambient scope (rather than in the global simple-name map).
    for candidate in lookup
        .ambient_symbols(&root.name)
        .iter()
        .filter(|candidate| is_namespace(&candidate.kind))
    {
        push_unique_module(&mut modules, &candidate.qualified_name);
    }
    modules
}

/// `true` when the chain's root segment names a namespace/module declaration
/// the current file binds. Exposed to the focused unit tests; production uses
/// [`namespace_root_modules`] to preserve the exact source-addressed modules.
#[cfg(test)]
fn chain_root_is_namespace(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
    lookup: &dyn SymbolLookup,
) -> bool {
    !namespace_root_modules(chain, file_ctx, file_package_id, lookup).is_empty()
}

/// A namespace declaration is import-bound only when the import's module can
/// actually reach it: the resolved module candidate, its workspace package,
/// its qname, or its indexed file path agrees with the import path.
fn namespace_matches_import(
    candidate: &crate::indexer::resolve::engine::contract::Symbol,
    import: &crate::indexer::resolve::engine::contract::ImportEntry,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> bool {
    let Some(module) = import.module_path.as_deref() else {
        return false;
    };
    lookup
        .in_module_from(&file_ctx.file_path, module)
        .iter()
        .any(|symbol| symbol.id == candidate.id)
        || lookup
            .workspace_package_id(module)
            .is_some_and(|package_id| candidate.package_id == Some(package_id))
        || candidate.qualified_name == module
        || super::support::qname_under_module(&candidate.qualified_name, module)
        || super::support::file_path_matches_module(&candidate.file_path, module)
}

/// Source-addressed modules named by a wildcard import root (`import * as v`).
fn wildcard_root_modules(chain: &crate::types::MemberChain, file_ctx: &FileContext) -> Vec<String> {
    let Some(root) = chain.segments.first() else {
        return Vec::new();
    };
    let mut modules = Vec::new();
    for import in &file_ctx.imports {
        if import.is_wildcard
            && (import.alias.as_deref() == Some(root.name.as_str())
                || import.imported_name == root.name)
        {
            if let Some(module) = import.module_path.as_deref() {
                push_unique_module(&mut modules, module);
            }
        }
    }
    modules
}

fn push_unique_module(modules: &mut Vec<String>, module: &str) {
    if !module.is_empty() && !modules.iter().any(|candidate| candidate == module) {
        modules.push(module.to_string());
    }
}

/// `true` when the chain's root segment names a wildcard/namespace import in this
/// file (`import * as v from 'm'` — `is_wildcard`, matched by alias or imported
/// name). The alias names a module, not a value, so `v.member` resolves against
/// the module's exports through the module-scoped ladder rather than the value walk.
#[cfg(test)]
fn chain_root_is_wildcard_import(
    chain: &crate::types::MemberChain,
    file_ctx: &FileContext,
) -> bool {
    let Some(root) = chain.segments.first() else {
        return false;
    };
    file_ctx.imports.iter().any(|i| {
        i.is_wildcard
            && (i.alias.as_deref() == Some(root.name.as_str()) || i.imported_name == root.name)
    })
}

/// Profile-table-driven kind compatibility. An unrecognised symbol-kind string
/// defaults permissive so an extractor typo doesn't silently hide a real symbol.
fn kind_ok_table(table: KindTable, edge: EdgeKind, sym_kind: &str) -> bool {
    match SymbolKind::from_str(sym_kind) {
        Ok(parsed) => KindCompatibility::check(table, edge, parsed),
        Err(_) => true,
    }
}

/// Test-only re-export of the engine's kind-compatibility predicate so sibling
/// tests can assert which symbol kinds a profile's `KindTable` admits for an
/// edge — the exact gate the rule ladder consults via `BinderContext.kind`.
#[cfg(test)]
pub(super) fn kind_ok_table_for_test(table: KindTable, edge: EdgeKind, sym_kind: &str) -> bool {
    kind_ok_table(table, edge, sym_kind)
}

#[cfg(test)]
#[path = "semantic_model_tests.rs"]
mod tests;
