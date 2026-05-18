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

use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::type_checker::alias::{build_alias_index, AliasIndex};
use crate::type_checker::core::chain::{ChainResolution, ChainWalker, DefaultRootResolver, RootResolver};
use crate::type_checker::core::inference::infer_expression_type;
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::{SymbolIdMap, SymbolTypeMap};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::ParsedFile;

/// Per-workspace engine state. Built once per indexing pass; `resolve` is
/// called per ref. Cheap to clone references out for parallel resolution
/// passes (`Engine` itself is not `Send` because `TypeArena` mutates on
/// resolve — the resolver loop wraps it in a per-worker handle).
pub struct Engine<'a> {
    arena: TypeArena,
    members: MembersIndex,
    supertypes: SupertypeGraph,
    symbol_types: SymbolTypeMap,
    aliases: AliasIndex,
    profiles: FxHashMap<&'static str, &'a LanguageProfile>,
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
    ) -> Engine<'static> {
        let mut profiles: FxHashMap<&'static str, &'static LanguageProfile> = FxHashMap::default();
        for plugin in crate::languages::default_registry().all() {
            let Some(profile) = plugin.profile() else {
                continue;
            };
            // Register the profile under every language id the plugin
            // claims so .tsx files get the TS profile, jsx gets the same
            // profile, Vue script blocks identified as "typescript" land
            // on the TS path, etc.
            for &lang in plugin.language_ids() {
                profiles.insert(lang, profile);
            }
        }
        Engine::build(parsed, sym_id_map, profiles, lookup)
    }

    pub fn build(
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        profiles: FxHashMap<&'static str, &'a LanguageProfile>,
        lookup: &dyn SymbolLookup,
    ) -> Self {
        let arena = TypeArena::new();

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
        }
    }

    /// Resolve a single ref. Routes through the unified chain walker when
    /// the ref carries a `MemberChain`; otherwise returns `None` (bare-name
    /// resolution stays in the legacy per-language resolver for now —
    /// Phase 5 migrates chain-bearing refs first, bare names follow as
    /// per-language hooks land).
    pub fn resolve(
        &self,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
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
        let resolution = walker.walk_with_root(chain, ref_ctx, file_ctx, &DefaultRootResolver)?;

        // Adapt ChainResolution -> legacy Resolution. The yield TypeId is
        // converted back to a qname string for legacy callers consuming
        // `resolved_yield_type: Option<String>`. Phase 9 swaps Resolution
        // to TypeId-native and this conversion drops out.
        Some(adapt_resolution(resolution, &self.arena))
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
        let resolution = walker.walk_with_root(chain, ref_ctx, file_ctx, root)?;
        Some(adapt_resolution(resolution, &self.arena))
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

/// Convert a TypeId-native ChainResolution into the legacy Resolution
/// shape the resolver loop still consumes. The yield type drops from
/// `TypeId` to `Option<String>` (the underlying qname) at the boundary.
fn adapt_resolution(cr: ChainResolution, arena: &TypeArena) -> Resolution {
    let yield_name = match arena.get(cr.resolved_yield_type) {
        Type::Class(q) => Some(q),
        Type::Apply { base, .. } => match arena.get(base) {
            Type::Class(q) => Some(q),
            _ => None,
        },
        Type::Optional(inner) => match arena.get(inner) {
            Type::Class(q) => Some(q),
            _ => None,
        },
        Type::AsyncWrapper(inner) => match arena.get(inner) {
            Type::Class(q) => Some(q),
            _ => None,
        },
        _ => None,
    };
    Resolution {
        target_symbol_id: cr.target_symbol_id,
        confidence: 1.0,
        strategy: cr.strategy,
        resolved_yield_type: yield_name,
        flow_emit: None,
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
