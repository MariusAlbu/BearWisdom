// =============================================================================
// type_checker/engine.rs — public Engine API
//
// Bundles every Phase 1-4 module into one façade: arena, members, supertypes,
// symbol_types, aliases, profiles. The resolver loop builds an Engine once
// per indexing pass (after symbols are persisted) and calls `resolve` for
// each ref. Languages whose plugins return `LanguagePlugin::profile()` are
// auto-registered; everything else is ignored.
//
// Phase 5 of the engine pivot.
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 5
//       research/architecture/04-implementation-phases.html § Phase 5
// =============================================================================

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::alias::{build_alias_index, AliasIndex};
use crate::type_checker::core::chain::{
    ChainResolution, ChainWalker, ProfileRootResolver, RootResolver,
};
use crate::type_checker::core::inference::infer_expression_type;
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::{SymbolIdMap, SymbolTypeMap};
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{EdgeKind, ParsedFile};

/// Kind-check seed for the generic resolver when the engine drives it: the
/// real gate is the profile's `kind_compatible_table`, applied via
/// `DefaultResolver::resolve_all_with_profile`. This fn-pointer slot is only
/// consulted by strategies the table-driven path doesn't reach (re-export
/// following), where permissive matches the prior bare-name behavior.
fn permissive_kind(_edge: EdgeKind, _sym_kind: &str) -> bool {
    true
}

/// Per-workspace engine state. Built once per indexing pass; `resolve` is
/// called per ref. The arena is shared via `Arc` with the
/// resolver's `SymbolIndex`, so TypeIds flowing from extractor →
/// SymbolIndex → Engine all reference the same canonical table.
pub struct Engine<'a> {
    arena: std::sync::Arc<TypeArena>,
    members: MembersIndex,
    supertypes: SupertypeGraph,
    symbol_types: SymbolTypeMap,
    aliases: AliasIndex,
    profiles: FxHashMap<&'static str, &'a LanguageProfile>,
    hooks: FxHashMap<&'static str, &'static dyn LanguageEngineHooks>,
}

impl<'a> Engine<'a> {
    /// Construct engine state from extraction output.
    ///
    /// Walks every `ParsedFile` once to build:
    ///   - SymbolTypeMap (with self-yielding reverse index for type-defining
    ///     symbols).
    ///   - MembersIndex (direct members keyed by parent's class TypeId).
    ///   - SupertypeGraph (Explicit / Structural / Both per profile).
    ///   - AliasIndex (TypeId → AliasTarget for every emitted alias pair).
    /// Auto-collect profiles from `crate::languages::default_registry()` and
    /// build engine state from those + the parsed files. The common entry
    /// point for the resolver loop.
    pub fn build_from_registry(
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        lookup: &dyn SymbolLookup,
        arena: std::sync::Arc<TypeArena>,
    ) -> Engine<'static> {
        let mut profiles: FxHashMap<&'static str, &'static LanguageProfile> = FxHashMap::default();
        let mut hooks: FxHashMap<&'static str, &'static dyn LanguageEngineHooks> =
            FxHashMap::default();
        for plugin in crate::languages::default_registry().all() {
            if let Some(profile) = plugin.profile() {
                // Register the profile under every language id the plugin
                // claims so .tsx files get the TS profile, jsx gets the
                // same profile, Vue script blocks identified as
                // "typescript" land on the TS path, etc.
                for &lang in plugin.language_ids() {
                    profiles.insert(lang, profile);
                }
            }
            if let Some(plugin_hooks) = plugin.language_hooks() {
                for &lang in plugin.language_ids() {
                    hooks.insert(lang, plugin_hooks);
                }
            }
        }
        Engine::build_with_hooks(parsed, sym_id_map, profiles, hooks, lookup, arena)
    }

    pub fn build(
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        profiles: FxHashMap<&'static str, &'a LanguageProfile>,
        lookup: &dyn SymbolLookup,
    ) -> Self {
        Self::build_with_hooks(
            parsed,
            sym_id_map,
            profiles,
            FxHashMap::default(),
            lookup,
            std::sync::Arc::new(TypeArena::new()),
        )
    }

    /// Build with explicit per-language hooks. Same as `build` but takes a
    /// hooks map keyed by language id. Hooks are stored on the engine and
    /// exposed via `hooks_for`. Decorator-driven and source-generator-driven
    /// members are surfaced by the per-language extractor (real symbols,
    /// FK-safe ids), not by post-build synthesis. The hook trait remains
    /// the seam for type-inference-only signals that don't need DB ids.
    pub fn build_with_hooks(
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        profiles: FxHashMap<&'static str, &'a LanguageProfile>,
        hooks: FxHashMap<&'static str, &'static dyn LanguageEngineHooks>,
        lookup: &dyn SymbolLookup,
        arena: std::sync::Arc<TypeArena>,
    ) -> Self {

        let members =
            MembersIndex::build_from_parsed_files(parsed, sym_id_map, &arena);
        let symbol_types = SymbolTypeMap::build_from_parsed_files(
            parsed,
            sym_id_map,
            &arena,
            // Use the first registered profile as the build profile. The
            // self-yield rule is language-agnostic so any profile suffices
            // here; per-language behavior switches at resolve time.
            profiles
                .values()
                .next()
                .copied()
                .unwrap_or(&crate::type_checker::profile::language_profile::DEFAULT_PROFILE),
        );

        // Supertype graph build depends on per-language profile. Pick the
        // first registered profile as a default; per-file resolves consult
        // the right profile via `Engine::resolve`.
        let default_profile = profiles
            .values()
            .next()
            .copied()
            .unwrap_or(&crate::type_checker::profile::language_profile::DEFAULT_PROFILE);
        let supertypes = SupertypeGraph::build(
            parsed,
            &arena,
            default_profile,
            &members,
            &symbol_types,
            lookup,
        );

        // Aliases: aggregate every per-file `(qname, AliasTarget)` pair.
        // Externals contribute alias entries that the chain walker resolves
        // *into* but doesn't walk from, so filter them out here too.
        let mut alias_pairs: Vec<(String, crate::types::AliasTarget)> = Vec::new();
        for pf in parsed {
            if pf.path.starts_with("ext:") {
                continue;
            }
            for (qname, target) in &pf.alias_targets {
                alias_pairs.push((qname.clone(), target.clone()));
            }
        }
        let aliases = build_alias_index(&alias_pairs, &arena);

        Self {
            arena,
            members,
            supertypes,
            symbol_types,
            aliases,
            profiles,
            hooks,
        }
    }

    /// Look up the engine hooks registered for `language`. Returns `None`
    /// when no hooks plugin opted in; engine then uses the no-op default
    /// for that language.
    pub fn hooks_for(
        &self,
        language: &str,
    ) -> Option<&'static dyn LanguageEngineHooks> {
        self.hooks.get(language).copied()
    }

    /// Resolve a single ref. Chain-bearing refs route through the unified
    /// chain walker; chain-less and single-segment refs route through the
    /// generic resolver's strategy ladder (`DefaultResolver`). When the
    /// engine path declines, falls through to the per-language hook resolver
    /// as a residue catch.
    pub fn resolve(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let lang = file_ctx.language.as_str();
        let profile = self.profiles.get(lang).copied();
        let hooks = self.hooks.get(lang).copied();

        // Chain-bearing refs route through the unified chain walker when
        // a profile is registered.
        if let Some(profile) = profile {
            if let Some(chain) = ref_ctx.extracted_ref.chain.as_ref() {
                let mut walker = ChainWalker::new(
                    &self.arena,
                    &self.members,
                    &self.supertypes,
                    &self.symbol_types,
                    &self.aliases,
                    profile,
                    lookup,
                );
                // Root resolution, most-specific first: a per-language
                // `root_resolver` hook wins when present (residue path), else
                // the profile-driven `ProfileRootResolver`. The profile path
                // reads `self_receiver_discovery` — `ScopePathThenDefault` (the
                // default) is byte-identical to `DefaultRootResolver`, and
                // `CanonicalMembers` discovers a framework `this` type the
                // extractor can't capture as a scope_path (Vue instance), so a
                // framework opts in through profile data rather than a hook.
                let profile_root = ProfileRootResolver::new(profile.self_receiver_discovery);
                let hook_root = hooks.and_then(|h| h.root_resolver());
                let root: &dyn RootResolver = match hook_root {
                    Some(r) => r,
                    None => &profile_root,
                };
                if let Some(cr) = walker.walk_with_root(chain, ref_ctx, file_ctx, root) {
                    return Some(Resolution {
                        target_symbol_id: cr.target_symbol_id,
                        confidence: 1.0,
                        strategy: cr.strategy,
                        resolved_yield_type: self.yield_or_none(cr.resolved_yield_type),
                        flow_emit: None,
                    });
                }
                // A single-segment "chain" (`foo()`, `Bar`) carries no receiver
                // type for the walker to root, so it always declines. Fall
                // through to the scope / same-file bare resolver — the same path
                // the chain-less arm below uses. Multi-segment chains that
                // decline are genuine misses and must NOT be re-probed by bare
                // name (Invariant #2: a same-name sibling could hijack `a.b.c`).
                if chain.segments.len() == 1 {
                    let bare = hooks
                        .and_then(|h| h.resolve_bare_pre(ref_ctx, file_ctx, lookup))
                        .or_else(|| self.resolve_generic(ref_ctx, file_ctx, lookup, profile))
                        .or_else(|| {
                            hooks.and_then(|h| h.resolve_bare_post(ref_ctx, file_ctx, lookup))
                        });
                    if let Some(mut r) = bare {
                        self.select_bare_overload_override(&mut r, ref_ctx, lookup, profile);
                        self.fill_bare_call_yield(&mut r, ref_ctx, lookup, profile);
                        return Some(r);
                    }
                }
            } else {
                // Chain-less ref: bare-name path. Hook pre-pass, then the
                // generic resolver's full strategy ladder, then the hook
                // post-pass; fill an argument-driven generic yield on
                // whichever succeeds so a `const x = genericFn(u)` binding
                // types `x` precisely (INFER-8 at the bare-name site).
                let bare = hooks
                    .and_then(|h| h.resolve_bare_pre(ref_ctx, file_ctx, lookup))
                    .or_else(|| self.resolve_generic(ref_ctx, file_ctx, lookup, profile))
                    .or_else(|| {
                        hooks.and_then(|h| h.resolve_bare_post(ref_ctx, file_ctx, lookup))
                    });
                if let Some(mut r) = bare {
                    self.select_bare_overload_override(&mut r, ref_ctx, lookup, profile);
                    self.fill_bare_call_yield(&mut r, ref_ctx, lookup, profile);
                    return Some(r);
                }
            }
        }

        // Engine path declined. Fall through to the not-yet-deleted per-language
        // hook resolver as the residue catch; chain-less hits still get an
        // argument-driven yield (chain refs already yield via the walker).
        let mut resolved = hooks.and_then(|h| h.resolve_ref(file_ctx, ref_ctx, lookup))?;
        if ref_ctx.extracted_ref.chain.is_none() {
            if let Some(profile) = profile {
                self.select_bare_overload_override(&mut resolved, ref_ctx, lookup, profile);
                self.fill_bare_call_yield(&mut resolved, ref_ctx, lookup, profile);
            }
        }
        Some(resolved)
    }

    /// Run the generic resolver's full strategy ladder for a chain-less or
    /// single-segment ref, gating candidate kinds against the language
    /// profile's `kind_compatible_table`.
    fn resolve_generic(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> Option<Resolution> {
        // Language builtins/primitives (scalar types, built-in functions,
        // operators, reserved namespace prefixes) are not project symbols.
        // Decline before the ladder so they are never bound to a same-named
        // project symbol and never seed a chain miss — Tier 1.5 classifies
        // them as external/builtin instead.
        if let Some(is_builtin) = profile.builtin_skip {
            if is_builtin(ref_ctx.extracted_ref.target_name.as_str()) {
                return None;
            }
        }
        // Two-key sibling of `builtin_skip`: a name reserved only inside a
        // specific kind of file. Declines before the ladder when the file's
        // namespace arms the rule AND the target is reserved, so a same-named
        // project symbol can't bind; external classification brands it after.
        if let Some(nd) = profile.namespace_decline {
            if file_ctx.file_namespace.as_deref() == Some(nd.file_namespace)
                && (nd.is_reserved)(ref_ctx.extracted_ref.target_name.as_str())
            {
                return None;
            }
        }
        // Import-prefix decline: a qualified target whose leading namespace
        // segment names a declared dependency module is external, not a project
        // symbol. Declines before the ladder so no same-named local binds and
        // external classification brands it. The import-set-keyed sibling of
        // `namespace_decline` — keyed on the target's own leading segment rather
        // than the resolving file's namespace.
        if profile.decline_qualified_when_prefix_imported {
            let target = ref_ctx.extracted_ref.target_name.as_str();
            let sep = profile.qname_separator;
            if let Some(head) = target.find(sep).map(|i| &target[..i]) {
                let bare_head = strip_leading_sigil(head, profile.self_keywords);
                if !bare_head.is_empty()
                    && file_ctx
                        .imports
                        .iter()
                        .any(|i| i.module_path.as_deref() == Some(bare_head))
                {
                    return None;
                }
            }
        }
        crate::type_checker::core::DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: permissive_kind,
        }
        .resolve_all_with_profile(profile)
    }

    /// Bare-name overload disambiguation. When a chain-less call resolved to one
    /// of several same-name callables in the same scope, re-select the target by
    /// argument arity (and type, when the arguments are concrete): if the call's
    /// arguments uniquely pick a single same-scope overload, retarget the
    /// resolution to it. A no-op unless the ref is a call carrying arguments; the
    /// scope + uniqueness gate lives on `ChainWalker` so the chain and bare paths
    /// share one filter. Runs BEFORE `fill_bare_call_yield` so the yield is
    /// computed against the corrected target.
    fn select_bare_overload_override(
        &self,
        r: &mut Resolution,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) {
        if ref_ctx.extracted_ref.call_args.is_empty()
            || !matches!(
                ref_ctx.extracted_ref.kind,
                EdgeKind::Calls | EdgeKind::Instantiates
            )
        {
            return;
        }
        // Recover the first-match symbol so the overload search is scoped to its
        // own enclosing scope, never a whole-program homonym.
        let target_id = r.target_symbol_id;
        let by_name = lookup.by_name(&ref_ctx.extracted_ref.target_name);
        let Some(current) = by_name.iter().find(|s| s.id == target_id) else {
            return;
        };
        let walker = ChainWalker::new(
            &self.arena,
            &self.members,
            &self.supertypes,
            &self.symbol_types,
            &self.aliases,
            profile,
            lookup,
        );
        if let Some(id) = walker.select_bare_overload(
            &ref_ctx.extracted_ref.target_name,
            &ref_ctx.extracted_ref.call_args,
            current,
        ) {
            if id != r.target_symbol_id {
                r.target_symbol_id = id;
                r.strategy = "bare_overload_arg_typed";
            }
        }
    }

    /// INFER-8 at the bare-name site. When a chain-less call resolved to a
    /// generic function/method whose declared return is one of its own type
    /// parameters, bind those parameters from the resolved argument types and
    /// set the resolution's yield to the substituted concrete return — so the
    /// forward-flow cache types `const x = genericFn(u)` precisely. A no-op
    /// unless the ref is a call carrying arguments and the resolution has no
    /// yield yet; the actual inference lives on `ChainWalker` so the chain and
    /// bare paths share one implementation.
    fn fill_bare_call_yield(
        &self,
        r: &mut Resolution,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) {
        if r.resolved_yield_type.is_some()
            || ref_ctx.extracted_ref.call_args.is_empty()
            || !matches!(
                ref_ctx.extracted_ref.kind,
                EdgeKind::Calls | EdgeKind::Instantiates
            )
        {
            return;
        }
        let target_id = r.target_symbol_id;
        let Some(sym) = lookup
            .by_name(&ref_ctx.extracted_ref.target_name)
            .iter()
            .find(|s| s.id == target_id)
        else {
            return;
        };
        let walker = ChainWalker::new(
            &self.arena,
            &self.members,
            &self.supertypes,
            &self.symbol_types,
            &self.aliases,
            profile,
            lookup,
        );
        r.resolved_yield_type =
            walker.infer_bare_call_yield(sym, &ref_ctx.extracted_ref.call_args);
    }

    /// Same as `resolve` but exposes a caller-supplied `RootResolver` so
    /// per-language hooks can override scope rules (Python `self`, Rust
    /// `&mut self`, C# `this`/`base`) without subclassing the engine.
    pub fn resolve_with_root(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        root: &dyn RootResolver,
    ) -> Option<Resolution> {
        let profile = self.profiles.get(file_ctx.language.as_str()).copied()?;
        let chain = ref_ctx.extracted_ref.chain.as_ref()?;

        let mut walker = ChainWalker::new(
            &self.arena,
            &self.members,
            &self.supertypes,
            &self.symbol_types,
            &self.aliases,
            profile,
            lookup,
        );
        let cr = walker.walk_with_root(chain, ref_ctx, file_ctx, root)?;
        Some(Resolution {
            target_symbol_id: cr.target_symbol_id,
            confidence: 1.0,
            strategy: cr.strategy,
            resolved_yield_type: self.yield_or_none(cr.resolved_yield_type),
            flow_emit: None,
        })
    }

    /// Map the chain walker's yield slot to a forward-inference type. A chain
    /// that resolves its target but can't type the last segment's yield
    /// returns `Type::Unknown` (the engine-bailout sentinel). Forward-flow
    /// inference must see `None` there so the resolver loop falls back to the
    /// target's declared return type — recording a local as `unknown` would
    /// poison every later member access on it.
    fn yield_or_none(&self, id: TypeId) -> Option<TypeId> {
        match self.arena.get(id) {
            crate::type_checker::core::types::Type::Unknown => None,
            _ => Some(id),
        }
    }

    /// Direct access to engine-side inference for refs without an attached
    /// resolution. Used by the resolver loop to populate the local-type
    /// cache when a flow-binding ref doesn't yet have a chain.
    pub fn infer_yield(
        &self,
        expr_ref: &crate::types::ExtractedRef,
        existing: Option<&Resolution>,
        language: &str,
    ) -> Option<TypeId> {
        let profile = self.profiles.get(language).copied()?;
        infer_expression_type(expr_ref, existing, &self.arena, profile)
    }

    /// Classify a ref as external via the language hook. Engine consults
    /// the hook registered for `file_ctx.language`; returns `None` when
    /// no language hook produced a classification (engine's generic
    /// external paths still run as a fallback).
    pub fn classify_external(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        project_ctx: Option<&crate::indexer::project_context::ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let hooks = self.hooks.get(file_ctx.language.as_str()).copied()?;
        hooks.classify_external(ref_ctx, file_ctx, project_ctx, lookup)
    }

    /// Detect cross-tier flow-emission patterns via the per-language hook.
    /// Returns an empty Vec when no hook is registered for the file's
    /// language. The legacy `LanguageResolver::detect_flow_emission_with_lookup`
    /// runs separately for languages not yet on hooks.
    pub fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let Some(hooks) = self.hooks.get(file_ctx.language.as_str()).copied() else {
            return Vec::new();
        };
        hooks.detect_flow_emissions(file_ctx, ref_ctx, lookup)
    }

    /// Build the per-file resolution context.
    ///
    /// Tries the per-language hook first (the escape hatch for languages whose
    /// context needs more than profile data can express). When no hook builds
    /// one, falls back to a generic context constructed from profile data — but
    /// ONLY for languages whose file context is fully expressible as profile
    /// data: those that opt into template-include resolution
    /// (`import_resolution.is_some()`) or harvest their import table from refs'
    /// `module` field (`import_module_path == FromModuleField`). Every other
    /// language keeps the prior contract of returning `None` when no hook builds
    /// a context, so the resolve loop's behavior for them is unchanged.
    pub fn build_file_context(
        &self,
        language: &str,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&crate::indexer::project_context::ProjectContext>,
    ) -> Option<FileContext> {
        use crate::type_checker::profile::language_profile::ImportModulePath;
        if let Some(hooks) = self.hooks.get(language).copied() {
            if let Some(ctx) = hooks.build_file_context(file, project_ctx) {
                return Some(ctx);
            }
        }
        let profile = self.profiles.get(language).copied()?;
        let expressible = profile.import_resolution.is_some()
            || profile.import_module_path == ImportModulePath::FromModuleField;
        if !expressible {
            return None;
        }
        Some(generic_file_context(language, file, profile))
    }

    /// Resolve a ref via the per-language hook. Returns `None` when no hook
    /// is registered or the hook itself declines.
    pub fn resolve_ref_via_hook(
        &self,
        language: &str,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let hooks = self.hooks.get(language).copied()?;
        hooks.resolve_ref(file_ctx, ref_ctx, lookup)
    }

    pub fn arena(&self) -> &TypeArena {
        &self.arena
    }

    pub fn members(&self) -> &MembersIndex {
        &self.members
    }

    pub fn supertypes(&self) -> &SupertypeGraph {
        &self.supertypes
    }

    pub fn symbol_types(&self) -> &SymbolTypeMap {
        &self.symbol_types
    }

    pub fn aliases(&self) -> &AliasIndex {
        &self.aliases
    }

    pub fn profile_for(&self, language: &str) -> Option<&LanguageProfile> {
        self.profiles.get(language).copied()
    }
}

/// Strip a leading variable sigil from a target's namespace head before the
/// import-prefix decline compares it against the import set. Removes a `$`
/// (the variable sigil) or any of the profile's `self_keywords` used as a bare
/// leading token, so `$foo::bar` and `foo::bar` compare identically against an
/// imported `foo` module.
fn strip_leading_sigil<'t>(head: &'t str, self_keywords: &[&str]) -> &'t str {
    let stripped = head.strip_prefix('$').unwrap_or(head);
    for kw in self_keywords {
        if stripped == *kw {
            return "";
        }
    }
    stripped
}

/// Generic `FileContext` built purely from profile data, for languages that
/// register a profile but no `build_file_context` hook. `import_module_path`
/// selects which refs become `ImportEntry`s and how `module_path` is filled:
///   - `None` — every `EdgeKind::Imports` ref, `module_path` empty.
///   - `EchoTarget` — every `Imports` ref, `module_path` = the raw target.
///   - `FromModuleField` — every ref (any edge kind) that carries a `module`
///     field, `module_path` = that module. The TS/JS extractor emits one
///     `TypeRef`-with-module ref per imported binding and attaches a `module`
///     to post-pass call refs; both are the file's import table.
/// `file_namespace` is `None` — namespace-bearing languages ship a hook.
fn generic_file_context(
    language: &str,
    file: &crate::types::ParsedFile,
    profile: &LanguageProfile,
) -> FileContext {
    use crate::type_checker::profile::language_profile::ImportModulePath;
    let imports: Vec<ImportEntry> = match profile.import_module_path {
        ImportModulePath::FromModuleField => file
            .refs
            .iter()
            .filter_map(|r| {
                let module = r.module.clone()?;
                Some(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(module),
                    alias: None,
                    is_wildcard: false,
                })
            })
            .collect(),
        mode => file
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: match mode {
                    ImportModulePath::None => None,
                    ImportModulePath::EchoTarget => Some(r.target_name.clone()),
                    ImportModulePath::FromModuleField => unreachable!(),
                },
                alias: None,
                is_wildcard: false,
            })
            .collect(),
    };
    FileContext {
        file_path: file.path.clone(),
        language: language.to_string(),
        imports,
        file_namespace: None,
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
