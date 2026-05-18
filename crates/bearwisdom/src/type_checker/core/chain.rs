// =============================================================================
// type_checker/core/chain.rs — unified TypeId chain walker
//
// One walker, every language. Iterates a `MemberChain` segment-by-segment,
// keeping a `current_ty: TypeId` plus a `GenericEnv` of in-scope type-param
// bindings. At each segment:
//   1. Expand the current type through `expand_alias_typed` so alias hops
//      collapse into their concrete heads / Applications.
//   2. Ask `MembersIndex::lookup` for the segment's named member.
//   3. Derive the member's *yielded* TypeId from `SymbolTypeMap` (return
//      type for callables, declared type for value-bearing kinds).
//   4. Run `substitute` against the env so a member typed as `T` resolves
//      to whatever the enclosing Apply layer bound to `T`.
//   5. If the new current type is itself an `Apply`, extend the env with
//      base's generic params → args bindings so deeper segments see
//      consistent substitutions.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 4
//       research/architecture/04-implementation-phases.html § Phase 4
//
// Known scope limitations honoured by Phase 5+ migrations:
//   - Optional-chaining (`a?.b`) preserves the chain miss/hit on `b` but the
//     yield type does not re-wrap in Optional. Affects flow-typing of the
//     downstream binding, not target-symbol resolution.
//   - Per-segment narrowings (FlowMeta.narrowings) are not consulted here;
//     the resolver loop in Phase 5 threads them via SymbolLookup's cursor.
//   - Dispatch axis enforcement: the walker uses receiver dispatch via
//     MembersIndex::lookup. MultiArg (R/Clojure) and ReturnType (Haskell)
//     callers invoke `dispatch::select_method` directly with a synthesised
//     DispatchQuery; their per-language hook will replace this lookup call
//     when Wave B migrations land.
// =============================================================================

use super::types::{Type, TypeArena, TypeId};
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolInfo, SymbolLookup};
use crate::type_checker::alias::{expand_alias_typed, AliasIndex};
use crate::type_checker::core::generics::{substitute, GenericEnv};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{ChainSegment, EdgeKind, MemberChain, SegmentKind};

/// Engine-side resolution result for a chain. Distinct from the legacy
/// `Resolution` (which still carries `Option<String>` for the yield); the
/// engine consumes TypeId throughout and an adapter (Phase 5) converts at
/// the boundary back to the legacy shape.
#[derive(Debug, Clone)]
pub struct ChainResolution {
    /// DB id of the resolved target symbol.
    pub target_symbol_id: i64,
    /// TypeId yielded by the final chain segment. Used by the chain
    /// walker's caller to populate the local-type cache for forward flow.
    pub resolved_yield_type: TypeId,
    /// Which root strategy produced this resolution. Diagnostic only.
    pub strategy: &'static str,
}

/// Pluggable resolver for the chain's first segment. Provided by the
/// caller so language-specific scope rules (Python `self`, R `~$`, Rust
/// `&mut self`, TS `this`) can dispatch without baking each into the
/// walker. The default impl handles SelfRef + bare identifiers via
/// `SymbolLookup::types_by_name`.
pub trait RootResolver {
    fn resolve(
        &self,
        seg: &ChainSegment,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        arena: &TypeArena,
        lookup: &dyn SymbolLookup,
    ) -> Option<TypeId>;
}

/// Default root resolver used by the gate tests and by languages whose
/// scope rules fit the engine defaults. Tries:
///   1. SelfRef → enclosing class from `source_symbol.scope_path`.
///   2. TypeAccess / NamespaceAccess / Construction → `arena.class(name)`.
///   3. Identifier → unique `lookup.types_by_name` hit, else fall back to
///      `arena.class(name)` so chain walks against globals still work.
pub struct DefaultRootResolver;

impl RootResolver for DefaultRootResolver {
    fn resolve(
        &self,
        seg: &ChainSegment,
        ref_ctx: &RefContext,
        _file_ctx: &FileContext,
        arena: &TypeArena,
        lookup: &dyn SymbolLookup,
    ) -> Option<TypeId> {
        match seg.kind {
            SegmentKind::SelfRef => {
                let scope = ref_ctx.source_symbol.scope_path.as_ref()?;
                Some(arena.class(scope))
            }
            SegmentKind::TypeAccess
            | SegmentKind::NamespaceAccess
            | SegmentKind::Construction => Some(arena.class(&seg.name)),
            SegmentKind::Identifier => {
                // 1. Local-variable inferred type wins over global symbols
                //    of the same name. `local_type` consults the per-file
                //    LocalTypeCache that the resolver loop populates as it
                //    encounters assignments; the cursor is moved before
                //    each ref so narrowings honour the current position.
                if let Some(local_qname) = lookup.local_type(&seg.name) {
                    return Some(arena.class(&local_qname));
                }
                // 2. Unique type symbol with this simple name.
                let matches = lookup.types_by_name(&seg.name);
                if matches.len() == 1 {
                    return Some(arena.class(&matches[0].qualified_name));
                }
                // 3. Fall back to interning the bare identifier as a class
                //    qname. Per-language scope tracking (Rust use, C#
                //    using, TS imports) plugs its own RootResolver to
                //    rewrite module-relative names before this fallback.
                Some(arena.class(&seg.name))
            }
            SegmentKind::Property | SegmentKind::ComputedAccess => None,
        }
    }
}

/// The unified chain walker. Owned references; consumers build a fresh
/// walker per chain (cheap — just a struct of references).
pub struct ChainWalker<'a> {
    pub arena: &'a mut TypeArena,
    pub members: &'a MembersIndex,
    pub supertypes: &'a SupertypeGraph,
    pub symbol_types: &'a SymbolTypeMap,
    pub aliases: &'a AliasIndex,
    pub profile: &'a LanguageProfile,
    pub lookup: &'a dyn SymbolLookup,
}

impl<'a> ChainWalker<'a> {
    pub fn new(
        arena: &'a mut TypeArena,
        members: &'a MembersIndex,
        supertypes: &'a SupertypeGraph,
        symbol_types: &'a SymbolTypeMap,
        aliases: &'a AliasIndex,
        profile: &'a LanguageProfile,
        lookup: &'a dyn SymbolLookup,
    ) -> Self {
        Self {
            arena,
            members,
            supertypes,
            symbol_types,
            aliases,
            profile,
            lookup,
        }
    }

    /// Resolve `chain` end-to-end. Returns `Some(ChainResolution)` only
    /// when *every* segment past the root resolves cleanly; a single miss
    /// collapses the whole chain to `None`. Callers that need partial
    /// progress diagnostics consult the per-segment loop directly via
    /// `walk_with_root`.
    pub fn walk(
        &mut self,
        chain: &MemberChain,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
    ) -> Option<ChainResolution> {
        self.walk_with_root(chain, ref_ctx, file_ctx, &DefaultRootResolver)
    }

    /// Walk with a caller-supplied root resolver.
    pub fn walk_with_root(
        &mut self,
        chain: &MemberChain,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
        root: &dyn RootResolver,
    ) -> Option<ChainResolution> {
        if chain.segments.is_empty() {
            return None;
        }
        let root_seg = &chain.segments[0];
        let mut current_ty = root.resolve(root_seg, ref_ctx, file_ctx, self.arena, self.lookup)?;
        current_ty = self.expand_aliases(current_ty);

        let mut env = GenericEnv::new();
        self.bind_apply_args(current_ty, &mut env);

        // Single-segment chains: the root IS the resolution. Use the
        // self-yielding reverse index to find the matching symbol id.
        // Falls through to the segment loop when there's more to walk.
        if chain.segments.len() == 1 {
            let target_sym_id = self.symbol_types.sym_id_for_class(current_ty)?;
            return Some(ChainResolution {
                target_symbol_id: target_sym_id,
                resolved_yield_type: current_ty,
                strategy: "engine_chain_root",
            });
        }

        let mut last_member: Option<SymbolInfo> = None;
        let last_idx = chain.segments.len() - 1;

        for (i, seg) in chain.segments.iter().enumerate().skip(1) {
            // Re-expand aliases in case the previous yield landed on one.
            current_ty = self.expand_aliases(current_ty);

            let kind_filter = if i == last_idx {
                ref_ctx.extracted_ref.kind
            } else {
                EdgeKind::TypeRef
            };

            let member = self.members.lookup(
                current_ty,
                &seg.name,
                kind_filter,
                self.supertypes,
                self.arena,
                self.profile,
            )?;

            let next_ty = self.yield_type_of(&member, seg, &env)?;
            current_ty = next_ty;
            self.bind_apply_args(current_ty, &mut env);
            last_member = Some(member);
        }

        let final_sym = last_member?;
        Some(ChainResolution {
            target_symbol_id: final_sym.id,
            resolved_yield_type: current_ty,
            strategy: "engine_chain",
        })
    }

    /// Derive the TypeId a chain segment advances to after resolving to
    /// `sym`. Methods/functions/constructors yield their return type;
    /// fields/properties/variables yield their declared type. Type-defining
    /// kinds yield self (callable class -> instance) when the segment is a
    /// Construction; otherwise they pass through as the type itself.
    fn yield_type_of(
        &mut self,
        sym: &SymbolInfo,
        seg: &ChainSegment,
        env: &GenericEnv,
    ) -> Option<TypeId> {
        let data = self.symbol_types.get(sym.id)?;
        let raw = match sym.kind.as_str() {
            "method" | "function" | "constructor" => data.return_type,
            "field" | "property" | "variable" | "parameter" | "enum_member" => {
                data.declared_type
            }
            "class" | "struct" | "interface" | "trait" | "enum" | "type_alias"
                if seg.kind == SegmentKind::Construction =>
            {
                data.return_type
            }
            _ => data.declared_type.or(data.return_type),
        }?;
        Some(substitute(raw, env, self.arena))
    }

    /// Walk `current_ty` through alias expansion until a non-alias head is
    /// reached or the expander returns `None`. Bounded by the alias
    /// expander's own depth cap.
    fn expand_aliases(&mut self, current_ty: TypeId) -> TypeId {
        let mut ty = current_ty;
        for _ in 0..8 {
            match expand_alias_typed(ty, self.arena, self.aliases, self.lookup) {
                Some(next) if next != ty => {
                    ty = next;
                }
                _ => break,
            }
        }
        ty
    }

    /// When `ty` is `Type::Apply { base, args }`, bind the base's declared
    /// generic params to `args` in `env` so subsequent segments substitute
    /// correctly. Bindings overwrite prior entries with the same id —
    /// matching the lexical-scope shadowing rule of nested generics.
    fn bind_apply_args(&mut self, ty: TypeId, env: &mut GenericEnv) {
        let (base, args) = match self.arena.get(ty).clone() {
            Type::Apply { base, args } => (base, args),
            _ => return,
        };
        if args.is_empty() {
            return;
        }
        let Some(data) = self.symbol_types.data_for_class(base) else {
            return;
        };
        if data.generic_params.is_empty() {
            return;
        }
        env.bind_positional(&data.generic_params, &args);
    }
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
