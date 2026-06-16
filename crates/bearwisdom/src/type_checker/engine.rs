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

use crate::indexer::resolve::legacy::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
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
use crate::types::{ChainSegment, EdgeKind, MemberChain, ParsedFile, SegmentKind};

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
        let mut members = {
            let _t = crate::indexer::phase_timer::scope("engine.members.build");
            MembersIndex::build_from_parsed_files(parsed, sym_id_map, &arena)
        };
        let symbol_types = {
            let _t = crate::indexer::phase_timer::scope("engine.symbol_types.build");
            SymbolTypeMap::build_from_parsed_files(
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
            )
        };

        // Per-file-profile supertype build: each file's discovery rule comes
        // from its own language's profile, and the structural pass runs only
        // over types declared in Structural/Both-profile files.
        let supertypes = {
            let _t = crate::indexer::phase_timer::scope("engine.supertypes.build_multi");
            SupertypeGraph::build_multi(parsed, &arena, &profiles, &members, &symbol_types, lookup)
        };

        // Second external-admission pass, now that the supertype graph exists:
        // admit the members of the external types an internal symbol reaches
        // (inheritance parents + referenced field/return/param types), so a
        // chain rooted on an external receiver walks past its first hop. The
        // trait/interface default-method admission from the first pass stands.
        {
            let _t = crate::indexer::phase_timer::scope("engine.admit_reachable_externals");
            members.admit_reachable_externals(parsed, sym_id_map, &arena, &supertypes);
        }

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

    /// Incrementally fold an expand iteration's appended `new_files` into the
    /// engine rather than rebuilding it from the full set. `parsed` is the FULL
    /// append-only set (`new_files` is its tail). The additive maps (members,
    /// symbol_types) ingest only `new_files` — byte-identical to a full rebuild
    /// because `parsed` is append-only, so per-parent member Vecs and
    /// sym_id-keyed type data land identically — while the cross-file structures
    /// (supertype graph's structural/blanket passes, alias index) are rebuilt
    /// over the full state, reading the now-complete members/symbol_types. The
    /// arena and registered profiles/hooks are unchanged. The return-inference
    /// loop appends no files and reuses the engine untouched: an inferred return
    /// has no extractor `return_type`, so `yield_type_of`'s view is `None` and
    /// it falls back to the lookup's `return_type_name`, which the SymbolIndex's
    /// `set_inferred_return` patches — the engine needs no change for it.
    pub fn augment(
        &mut self,
        parsed: &[ParsedFile],
        new_files: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        lookup: &dyn SymbolLookup,
    ) {
        if new_files.is_empty() {
            return;
        }
        let _t_augment = crate::indexer::phase_timer::scope("engine.augment");
        let default_profile = self
            .profiles
            .values()
            .next()
            .copied()
            .unwrap_or(&crate::type_checker::profile::language_profile::DEFAULT_PROFILE);

        {
            let _t = crate::indexer::phase_timer::scope("engine.augment.members.ingest");
            self.members
                .ingest_files(new_files, sym_id_map, &self.arena);
        }
        {
            let _t = crate::indexer::phase_timer::scope("engine.augment.symbol_types.ingest");
            self.symbol_types
                .ingest_files(new_files, sym_id_map, &self.arena, default_profile);
        }

        {
            let _t = crate::indexer::phase_timer::scope("engine.augment.supertypes.build_multi");
            self.supertypes = SupertypeGraph::build_multi(
                parsed,
                &self.arena,
                &self.profiles,
                &self.members,
                &self.symbol_types,
                lookup,
            );
        }

        // Re-run the reachability-bounded external admission over the full set:
        // appended files may reference new external types, and the rebuilt
        // graph may add internal → external inheritance edges. The pass skips
        // owners already keyed, so it only admits the newly-reached types.
        {
            let _t = crate::indexer::phase_timer::scope("engine.augment.admit_reachable_externals");
            self.members
                .admit_reachable_externals(parsed, sym_id_map, &self.arena, &self.supertypes);
        }

        let mut alias_pairs: Vec<(String, crate::types::AliasTarget)> = Vec::new();
        for pf in parsed {
            if pf.path.starts_with("ext:") {
                continue;
            }
            for (qname, target) in &pf.alias_targets {
                alias_pairs.push((qname.clone(), target.clone()));
            }
        }
        self.aliases = build_alias_index(&alias_pairs, &self.arena);
    }

    /// Look up the engine hooks registered for `language`. Returns `None`
    /// when no hooks plugin opted in; engine then uses the no-op default
    /// for that language.
    pub fn hooks_for(&self, language: &str) -> Option<&'static dyn LanguageEngineHooks> {
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
                        .or_else(|| self.resolve_via_adl(ref_ctx, lookup, profile))
                        .or_else(|| self.resolve_via_arity_ranked(ref_ctx, lookup, profile))
                        .or_else(|| {
                            hooks.and_then(|h| h.resolve_bare_post(ref_ctx, file_ctx, lookup))
                        });
                    if let Some(mut r) = bare {
                        self.select_bare_overload_override(&mut r, ref_ctx, lookup, profile);
                        self.fill_bare_call_yield(&mut r, ref_ctx, lookup, profile);
                        return Some(r);
                    }
                    // The bare ladder declined. If the enclosing type inherits,
                    // retry the bare name as a synthetic `self.name()` chain so
                    // an inherited member can bind through the walker.
                    if let Some(r) =
                        self.try_implicit_self_member(ref_ctx, file_ctx, lookup, profile)
                    {
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
                    .or_else(|| self.resolve_via_adl(ref_ctx, lookup, profile))
                    .or_else(|| self.resolve_via_arity_ranked(ref_ctx, lookup, profile))
                    .or_else(|| hooks.and_then(|h| h.resolve_bare_post(ref_ctx, file_ctx, lookup)));
                if let Some(mut r) = bare {
                    self.select_bare_overload_override(&mut r, ref_ctx, lookup, profile);
                    self.fill_bare_call_yield(&mut r, ref_ctx, lookup, profile);
                    return Some(r);
                }
                // The bare ladder declined. If the enclosing type inherits,
                // retry the bare name as a synthetic `self.name()` chain so an
                // inherited member can bind through the walker.
                if let Some(r) = self.try_implicit_self_member(ref_ctx, file_ctx, lookup, profile) {
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
        if self.bare_decline(ref_ctx, file_ctx, profile) {
            return None;
        }
        crate::type_checker::core::DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: permissive_kind,
        }
        .resolve_all_with_profile(profile)
    }

    /// True when the profile deliberately declines this bare target before the
    /// ladder ever runs — a builtin/primitive, a name reserved inside the file's
    /// namespace, or a qualified target whose leading segment names an imported
    /// dependency. A `true` here is a decision, not a miss: the target is
    /// external/builtin and no project symbol may bind it. Consulted both by
    /// `resolve_generic` (so the ladder skips it) and by the implicit-self
    /// synthesis (so a declined name is never resurrected as an inherited member).
    fn bare_decline(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        profile: &LanguageProfile,
    ) -> bool {
        // Language builtins/primitives (scalar types, built-in functions,
        // operators, reserved namespace prefixes) are not project symbols, so
        // they are never bound to a same-named project symbol and never seed a
        // chain miss — Tier 1.5 classifies them as external/builtin instead.
        if let Some(is_builtin) = profile.builtin_skip {
            if is_builtin(ref_ctx.extracted_ref.target_name.as_str()) {
                return true;
            }
        }
        // Two-key sibling of `builtin_skip`: a name reserved only inside a
        // specific kind of file. Declines when the file's namespace arms the
        // rule AND the target is reserved, so a same-named project symbol can't
        // bind; external classification brands it after.
        if let Some(nd) = profile.namespace_decline {
            if file_ctx.file_namespace.as_deref() == Some(nd.file_namespace)
                && (nd.is_reserved)(ref_ctx.extracted_ref.target_name.as_str())
            {
                return true;
            }
        }
        // Import-prefix decline: a qualified target whose leading namespace
        // segment names a declared dependency module is external, not a project
        // symbol. The import-set-keyed sibling of `namespace_decline` — keyed on
        // the target's own leading segment rather than the resolving file's
        // namespace.
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
                    return true;
                }
            }
        }
        false
    }

    /// Last-resort bare-call rung: an inherited-member retry for a chainless
    /// call the ladder declined. A bare `helper()` inside a method of a class
    /// that `extends`/`implements` a base never reaches the ChainWalker — it has
    /// no receiver — so an inherited member (internal or reachable-external base)
    /// can't bind. When the enclosing type has supertype edges, re-issue the
    /// bare name as a synthetic `self.name(...)` chain through the walker so its
    /// supertype walk + qname-keyed inheritance fallback find the member.
    ///
    /// Gating, in order:
    ///   - the ref is a `Calls`/`Instantiates` (member/call-shaped) bare target;
    ///   - the profile did NOT `bare_decline` it (a builtin/namespace/import
    ///     decline must still win — never resurrect a deliberately-declined name);
    ///   - the enclosing symbol's `scope_path` names a type that has supertype
    ///     edges (`SupertypeGraph::parents_of`), so a plain free function — whose
    ///     enclosing scope is absent or names a supertype-less owner — never
    ///     pays the walk, and flat-global languages keep their semantics.
    /// A success carries the `implicit_self_member` strategy so the synthesized
    /// origin is legible; the resolution is otherwise shaped exactly as an
    /// explicit `self.name()` chain would produce.
    fn try_implicit_self_member(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> Option<Resolution> {
        let er = ref_ctx.extracted_ref;
        // Member/call-shaped only. A bare TypeRef to an inherited nested type is
        // a different case and is not retried here.
        if !matches!(er.kind, EdgeKind::Calls | EdgeKind::Instantiates) {
            return None;
        }
        // A qualified target already carries its own receiver — the implicit
        // self is for an unqualified bare name.
        let target = er.target_name.as_str();
        if target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }
        // Never resurrect a name the profile deliberately declines.
        if self.bare_decline(ref_ctx, file_ctx, profile) {
            return None;
        }
        // The enclosing type must have supertype edges — otherwise an own-class
        // member would already have bound via the ladder, and a free function
        // has no enclosing type to walk.
        let scope = ref_ctx.source_symbol.scope_path.as_deref().filter(|s| !s.is_empty())?;
        let enclosing_ty = self.arena.class(scope);
        if self.supertypes.parents_of(enclosing_ty).is_empty() {
            return None;
        }

        // Synthesize `self.name(args)`: a `SelfRef` root the walker types to the
        // enclosing class from `scope_path` (same mechanism as an explicit
        // `self` receiver), then a member segment carrying the call's shape so
        // the terminal-call yield / overload logic matches `self.name()`.
        let chain = MemberChain {
            segments: vec![
                ChainSegment {
                    name: "self".to_string(),
                    node_kind: "self".to_string(),
                    kind: SegmentKind::SelfRef,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                    byte_offset: 0,
                    declared_type_id: None,
                    is_call: false,
                    call_args: Vec::new(),
                    type_arg_ids: Vec::new(),
                },
                ChainSegment {
                    name: target.to_string(),
                    node_kind: "property_identifier".to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: false,
                    byte_offset: 0,
                    declared_type_id: None,
                    is_call: true,
                    call_args: er.call_args.clone(),
                    type_arg_ids: Vec::new(),
                },
            ],
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
        let profile_root = ProfileRootResolver::new(profile.self_receiver_discovery);
        let cr = walker.walk_with_root(&chain, ref_ctx, file_ctx, &profile_root)?;
        Some(Resolution {
            target_symbol_id: cr.target_symbol_id,
            confidence: 1.0,
            strategy: "implicit_self_member",
            resolved_yield_type: self.yield_or_none(cr.resolved_yield_type),
            flow_emit: None,
        })
    }

    /// Argument-dependent lookup (ADL). A strict fallback for a bare call the
    /// regular ladder declined: resolve `swap(a, b)` to a free function `swap`
    /// declared in the namespace of one of its argument types, even though no
    /// import / scope / using brought `swap` into scope.
    ///
    /// Runs only when the profile opts in (`argument_dependent_lookup`), the ref
    /// is a `Calls`/`Instantiates` call carrying arguments, and the target is
    /// bare (no `.`/`::`/`/` qualifier). For each argument typed as a `Class`
    /// (or `Apply` over one) with a dotted qname, the declaring-namespace prefix
    /// (`rsplit_once('.')`) supplies a candidate `{namespace}.{target}`. A single
    /// unique candidate qname binds directly; several hand off to BIND-4's
    /// arity/type assignability filter and bind only the unique survivor — zero
    /// or multiple survivors decline (never guess).
    fn resolve_via_adl(
        &self,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> Option<Resolution> {
        if !profile.argument_dependent_lookup {
            return None;
        }
        let er = ref_ctx.extracted_ref;
        if !matches!(er.kind, EdgeKind::Calls | EdgeKind::Instantiates) || er.call_args.is_empty() {
            return None;
        }
        let target = er.target_name.as_str();
        if target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }

        // Type each argument, then collect the declaring namespace of every
        // argument typed as a nominal class (looking through one `Apply` layer).
        let arg_type_ids = crate::type_checker::core::dispatch::resolve_arg_types(
            &er.call_args,
            &self.arena,
            lookup,
            profile,
        );
        let mut candidates: Vec<SymbolInfo> = Vec::new();
        for &ty in &arg_type_ids {
            let Some(ns) = self.declaring_namespace_of(ty) else {
                continue;
            };
            let qname = format!("{ns}.{target}");
            for cand in lookup.all_by_qualified_name(&qname) {
                if !matches!(cand.kind.as_str(), "function" | "method" | "constructor") {
                    continue;
                }
                if candidates.iter().any(|c| c.id == cand.id) {
                    continue;
                }
                candidates.push(cand.clone());
            }
        }

        let chosen = match candidates.len() {
            0 => return None,
            // A single namespace-mate IS the structural answer — its namespace
            // owns the function. Arity isn't even consulted (a free function and
            // a same-named overload would both surface as candidates; one means
            // no ambiguity to resolve).
            1 => candidates.into_iter().next().unwrap(),
            // Several namespaces contributed same-name callables — reuse BIND-4's
            // sound arity/type filter and bind only when exactly one survives.
            _ => {
                let matches = crate::type_checker::core::dispatch::arg_assignable_candidates(
                    candidates,
                    &arg_type_ids,
                    &self.members,
                    &self.symbol_types,
                    &self.arena,
                    lookup,
                    profile,
                );
                if matches.len() != 1 {
                    return None;
                }
                matches.into_iter().next().unwrap()
            }
        };

        Some(Resolution {
            target_symbol_id: chosen.id,
            confidence: 1.0,
            strategy: "engine_adl",
            resolved_yield_type: None,
            flow_emit: None,
        })
    }

    /// Arity/type-driven overload pick for a bare call the structural ladder
    /// (including the profile's ranked-candidate scoring rung) declined. When a
    /// bare `Calls`/`Instantiates` carrying arguments has SEVERAL same-name,
    /// callable, project-internal candidates, type each argument and filter the
    /// set through the same assignability check the multi-arg dispatcher uses
    /// (`arg_assignable_candidates`, which rejects on arity first). A single
    /// survivor binds; zero or several decline (never guess).
    ///
    /// The argument-typed companion to `resolve_via_ranked_candidates`: that
    /// rung scores candidates the argument signal can't separate (a type used
    /// function-style, an argument-less call); this one lets concrete arguments
    /// pick the matching overload from a same-name set. Gated on
    /// `multi_candidate_ranking`; off (every non-opted language) is inert, and
    /// it runs only after `resolve_generic` declines, so every structural and
    /// scoring rung wins first.
    fn resolve_via_arity_ranked(
        &self,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
        profile: &LanguageProfile,
    ) -> Option<Resolution> {
        if !profile.multi_candidate_ranking {
            return None;
        }
        let er = ref_ctx.extracted_ref;
        if !matches!(er.kind, EdgeKind::Calls | EdgeKind::Instantiates) || er.call_args.is_empty() {
            return None;
        }
        let target = er.target_name.as_str();
        if target.contains('.') || target.contains("::") || target.contains('/') {
            return None;
        }
        // Same-name, callable, project-internal candidates. Constructors are
        // included so a type used function-style resolves to its constructor
        // overload when arguments separate them.
        let candidates: Vec<SymbolInfo> = lookup
            .by_name(target)
            .iter()
            .filter(|s| !lookup.is_external_file(&s.file_path))
            .filter(|s| matches!(s.kind.as_str(), "function" | "method" | "constructor"))
            .cloned()
            .collect();
        if candidates.len() < 2 {
            // Zero or one: the single-candidate / scoring rungs already had their
            // chance. Arity has nothing to disambiguate.
            return None;
        }
        let arg_type_ids = crate::type_checker::core::dispatch::resolve_arg_types(
            &er.call_args,
            &self.arena,
            lookup,
            profile,
        );
        let matches = crate::type_checker::core::dispatch::arg_assignable_candidates(
            candidates,
            &arg_type_ids,
            &self.members,
            &self.symbol_types,
            &self.arena,
            lookup,
            profile,
        );
        if matches.len() != 1 {
            return None;
        }
        let chosen = matches.into_iter().next().unwrap();
        Some(Resolution {
            target_symbol_id: chosen.id,
            confidence: 1.0,
            strategy: "engine_arity_ranked",
            resolved_yield_type: None,
            flow_emit: None,
        })
    }

    /// The declaring-namespace prefix of a nominal type: for `Class(qname)` (or
    /// an `Apply` whose base is a `Class`), the segment before the qname's final
    /// `.`. Returns `None` for an unqualified qname (no namespace to probe) or a
    /// non-nominal type. Qnames are `.`-joined by the scope-tree qualifier for
    /// every language, so the split is uniform.
    fn declaring_namespace_of(&self, ty: TypeId) -> Option<String> {
        let qname = match self.arena.get(ty) {
            crate::type_checker::core::types::Type::Class(q) => q,
            crate::type_checker::core::types::Type::Apply { base, .. } => {
                match self.arena.get(base) {
                    crate::type_checker::core::types::Type::Class(q) => q,
                    _ => return None,
                }
            }
            _ => return None,
        };
        qname.rsplit_once('.').map(|(ns, _)| ns.to_string())
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
            .into_iter()
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
        r.resolved_yield_type = walker.infer_bare_call_yield(sym, &ref_ctx.extracted_ref.call_args);
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
                    is_wildcard: r.target_name == "*",
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
                is_wildcard: r.target_name == "*",
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
