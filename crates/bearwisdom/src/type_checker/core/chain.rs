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
        file_ctx: &FileContext,
        arena: &TypeArena,
        lookup: &dyn SymbolLookup,
    ) -> Option<TypeId> {
        match seg.kind {
            SegmentKind::SelfRef => {
                if let Some(scope) = ref_ctx.source_symbol.scope_path.as_ref() {
                    return Some(arena.class(scope));
                }
                // Fallback when the source symbol carries no scope_path.
                // Common in extractors that emit a per-file class symbol
                // for the unit's primary type (Vue SFC, Astro page) but
                // do not reparent sibling helpers / Options-API
                // methods under it — `methods: { foo() {} }` lands `foo`
                // at file scope, and `this` in its body has no
                // class scope to attach to.
                //
                // Rule: when the source symbol's file declares exactly
                // one top-level type symbol (class/interface/struct/
                // namespace, scope_path empty or absent), treat `this`
                // as referring to that type. The "single top-level"
                // condition keeps the fallback from over-matching in
                // ordinary files that happen to declare several
                // unrelated classes.
                // Two-pass: prefer the type symbol whose simple name
                // matches the file stem (the SFC / page-class convention
                // — `EditorImage.vue` → `EditorImage`, `index.vue` →
                // `index`). Only when no stem match exists do we accept
                // a single-candidate fallback. Identifier-shape filtering
                // rejects CSS-selector noise from `<style>` blocks that
                // the SCSS extractor emits with `SymbolKind::Class`
                // (`editor-slide-upload`, `vue-image-crop-upload`,
                // `&:hover`).
                let stem = file_stem_of(&file_ctx.file_path);
                let mut stem_match: Option<&str> = None;
                let mut single_match: Option<&str> = None;
                let mut single_ambiguous = false;
                for sym in lookup.in_file(&file_ctx.file_path) {
                    let is_top_level = sym
                        .scope_path
                        .as_deref()
                        .map(|s| s.is_empty())
                        .unwrap_or(true);
                    if !is_top_level {
                        continue;
                    }
                    if !matches!(
                        sym.kind.as_str(),
                        "class" | "interface" | "struct" | "namespace"
                    ) {
                        continue;
                    }
                    if !is_identifier_like(&sym.name) {
                        continue;
                    }
                    if let Some(s) = stem.as_deref() {
                        if sym.name == s {
                            stem_match = Some(&sym.qualified_name);
                            break;
                        }
                    }
                    if single_match.is_some() {
                        single_ambiguous = true;
                    } else {
                        single_match = Some(&sym.qualified_name);
                    }
                }
                stem_match
                    .or(if single_ambiguous { None } else { single_match })
                    .map(|qname| arena.class(qname))
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
                // 3. Variable / parameter / field declared in an enclosing
                //    scope. For each scope walking outward, try the qualified
                //    name `{scope}.{name}` and consult the declared-type
                //    map. The map is keyed by qname and built from extractor
                //    output (signature parsing + TypeRef return-type pass)
                //    so this hop covers method parameters, instance fields,
                //    and type-annotated locals that didn't reach the
                //    LocalTypeCache yet.
                for scope in &ref_ctx.scope_chain {
                    let qname = format!("{scope}.{}", seg.name);
                    if let Some(type_name) = lookup.field_type_name(&qname) {
                        // When the declared type names a generic parameter of
                        // this scope, resolve to the canonical Type::Generic —
                        // which carries the param's upper bound — rather than a
                        // class literally named `T`. `t: T` in `f<T: Animal>`
                        // then finds members on Animal via the bound.
                        if let Some(ids) = lookup.generic_param_type_ids(scope) {
                            if let Some(&gid) =
                                ids.iter().find(|&&id| arena.format_type(id) == type_name)
                            {
                                return Some(gid);
                            }
                        }
                        return Some(arena.class(type_name));
                    }
                }
                // 4. Bare-specifier import binding. `import { vi } from 'vitest'`
                //    brings `vi` into scope as a value whose type lives at
                //    `vitest.vi`'s field_type / return_type. Iterate the file's
                //    imports for entries matching seg.name (by imported_name or
                //    alias), skip relative / absolute specifiers (those are
                //    project-internal and the scope-chain walk handles them),
                //    and probe `{module}.{name}` for a declared type. Covers
                //    `import dayjs from 'dayjs'; dayjs(...).toDate()` and the
                //    named-import pattern across TS / Python / Java.
                for import in &file_ctx.imports {
                    let matches_import = import.imported_name == seg.name
                        || import.alias.as_deref() == Some(seg.name.as_str());
                    if !matches_import {
                        continue;
                    }
                    let Some(module) = import.module_path.as_deref() else {
                        continue;
                    };
                    if module.starts_with('.') || module.starts_with('/') {
                        continue;
                    }
                    let candidate = format!("{module}.{}", seg.name);
                    if let Some(rt) = lookup.return_type_name(&candidate) {
                        return Some(arena.class(rt));
                    }
                    if let Some(ft) = lookup.field_type_name(&candidate) {
                        return Some(arena.class(ft));
                    }
                }
                // 5. npm globals fallback. When a project enables vitest /
                //    jest `globals: true`, identifiers like `vi`, `expect`,
                //    `describe`, `test` enter the file's scope without an
                //    explicit import. The npm ecosystem walker writes their
                //    type info under the synthetic `__npm_globals__.{name}`
                //    qname; this probe surfaces it. The qname constant lives
                //    in the npm ecosystem module — keeping the prefix as a
                //    crate-private literal here avoids leaking the
                //    convention into the SymbolLookup trait.
                let globals_candidate =
                    format!("{}.{}", crate::ecosystem::npm::NPM_GLOBALS_MODULE, seg.name);
                if let Some(rt) = lookup.return_type_name(&globals_candidate) {
                    return Some(arena.class(rt));
                }
                if let Some(ft) = lookup.field_type_name(&globals_candidate) {
                    return Some(arena.class(ft));
                }
                // 6. Fall back to interning the bare identifier as a class
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
    pub arena: &'a TypeArena,
    pub members: &'a MembersIndex,
    pub supertypes: &'a SupertypeGraph,
    pub symbol_types: &'a SymbolTypeMap,
    pub aliases: &'a AliasIndex,
    pub profile: &'a LanguageProfile,
    pub lookup: &'a dyn SymbolLookup,
}

impl<'a> ChainWalker<'a> {
    pub fn new(
        arena: &'a TypeArena,
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
        &self,
        chain: &MemberChain,
        ref_ctx: &RefContext,
        file_ctx: &FileContext,
    ) -> Option<ChainResolution> {
        self.walk_with_root(chain, ref_ctx, file_ctx, &DefaultRootResolver)
    }

    /// Walk with a caller-supplied root resolver.
    pub fn walk_with_root(
        &self,
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

            let member = match self.members.lookup_with_binding(
                current_ty,
                &seg.name,
                kind_filter,
                self.supertypes,
                self.arena,
                self.profile,
            ) {
                Some((m, owner, owner_args)) => {
                    // Inherited generic method: bind the ancestor's generic
                    // params to the arguments named on the `extends` /
                    // `implements` edge (`Repository<User>`), so a member
                    // returning `T` substitutes to the concrete type at the
                    // yield step below.
                    if !owner_args.is_empty() {
                        if let Some(data) = self.symbol_types.data_for_class(owner) {
                            if !data.generic_params.is_empty() {
                                env.bind_positional(&data.generic_params, &owner_args);
                            }
                        }
                    }
                    m
                }
                None => self.qualified_member_lookup(current_ty, &seg.name, file_ctx)?,
            };

            match self.yield_type_of(&member, seg, &env, current_ty) {
                Some(next_ty) => {
                    current_ty = next_ty;
                    self.bind_apply_args(current_ty, &mut env);
                }
                None if i == last_idx => {
                    // Last segment is the resolution target itself — its
                    // sym_id is already captured in `member`. A missing
                    // yield type is irrelevant to the edge this resolution
                    // produces. Set current_ty to Unknown so the legacy
                    // `resolved_yield_type` slot converts to None at the
                    // adapter boundary; forward-flow inference must not
                    // propagate the receiver type as if it were the
                    // member's return type.
                    current_ty = self.arena.intern(crate::type_checker::core::types::Type::Unknown);
                }
                None => return None,
            }
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
    ///
    /// Primary path: SymbolTypeMap, keyed by sym_id with rich TypeId data
    /// and generic substitution via `substitute`. Fallback: SymbolLookup's
    /// qname-keyed `field_type_name` / `return_type_name`, which are
    /// populated from signature parsing and TypeRef edges by build.rs.
    /// The fallback covers extractors that don't yet emit
    /// `ExtractedSymbol.return_type` / `declared_type` TypeIds (C# methods,
    /// Java fields, etc.) and bridges the engine to the rich string-keyed
    /// type info the legacy chain walkers consume. The string is decomposed
    /// via `intern_type_str` so a generic return keeps its `Apply` args;
    /// env-bound params then resolve through `substitute`, matching the
    /// SymbolTypeMap path.
    fn yield_type_of(
        &self,
        sym: &SymbolInfo,
        seg: &ChainSegment,
        env: &GenericEnv,
        current_ty: TypeId,
    ) -> Option<TypeId> {
        let data_raw = self.symbol_types.get(sym.id).and_then(|data| {
            match sym.kind.as_str() {
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
            }
        });
        if let Some(raw) = data_raw {
            return Some(substitute(raw, env, self.arena));
        }

        let raw_str = match sym.kind.as_str() {
            "method" | "function" | "constructor" => {
                self.lookup.return_type_name(&sym.qualified_name)
            }
            "field" | "property" | "variable" | "parameter" | "enum_member" => {
                self.lookup.field_type_name(&sym.qualified_name)
            }
            "class" | "struct" | "interface" | "trait" | "enum" | "type_alias"
                if seg.kind == SegmentKind::Construction =>
            {
                self.lookup.return_type_name(&sym.qualified_name)
            }
            _ => self
                .lookup
                .return_type_name(&sym.qualified_name)
                .or_else(|| self.lookup.field_type_name(&sym.qualified_name)),
        }?;
        // Fluent `: this` return: TypeScript / Kotlin / C++ method signatures
        // ending `(...): this` pin the chain's receiver type forward so
        // builder patterns (`new DocumentBuilder().setTitle().setVersion()`)
        // resolve every step against the same receiver. Apply only when the
        // resolved member is a callable; field/property `this` aliases are
        // not a real TS pattern.
        let raw_trim = raw_str.trim();
        let is_this_return = raw_trim == "this"
            && matches!(
                sym.kind.as_str(),
                "method" | "function" | "constructor"
            );
        if is_this_return {
            return Some(current_ty);
        }
        let cleaned = raw_trim.trim_end_matches('.');
        if cleaned.is_empty() {
            return None;
        }
        // Decompose the declared-type string structurally so a generic return
        // (`Iter<User>`, `Array<T>`) keeps its args instead of collapsing to
        // the bare base — the next segment can then carry or substitute them.
        // Mirrors the SymbolTypeMap path's `substitute` step; concrete args
        // (`Iter<User>`) survive directly, env-bound params (`<T>`) resolve.
        let yielded = self.arena.intern_type_str(cleaned);
        Some(substitute(yielded, env, self.arena))
    }

    /// Qualified-name fallback for member lookup when MembersIndex misses.
    /// Constructs `{current_ty_qname}.{seg_name}` and consults
    /// `SymbolLookup::by_qualified_name`. Covers chains targeting external
    /// symbols (externals are excluded from MembersIndex at build time to
    /// keep the engine arena bounded, but their qnames live in the global
    /// symbol index) and intra-project members reachable by qname but not
    /// reachable from `current_ty`'s direct/extension maps (cross-language
    /// shims, embedded-region symbols, etc.). Returns the same
    /// `SymbolInfo` shape MembersIndex returns so the walker downstream
    /// can't tell which path produced the member.
    fn qualified_member_lookup(
        &self,
        current_ty: TypeId,
        seg_name: &str,
        file_ctx: &FileContext,
    ) -> Option<SymbolInfo> {
        let qname = match self.arena.get(current_ty) {
            Type::Class(q) => q,
            Type::Apply { base, .. } => match self.arena.get(base) {
                Type::Class(q) => q,
                _ => return None,
            },
            _ => return None,
        };
        let candidate = format!("{qname}.{seg_name}");
        if let Some(hit) = self.lookup.by_qualified_name(&candidate) {
            return Some(hit.clone());
        }
        // External-type qname promotion. When current_ty's qname is a short
        // name shadowed by an external library type ("Assertion" lives as
        // "chai.Assertion", "Subject" as "rxjs.Subject", "JsonConvert" as
        // "Newtonsoft.Json.JsonConvert"), the direct qname probe misses
        // because the member is keyed under the external's package-prefixed
        // qname. Find every external type sharing this short name and
        // retry the member lookup against each external qname.
        let short_name = qname.rsplit('.').next().unwrap_or(qname.as_str());
        for candidate_type in self.lookup.types_by_name(short_name) {
            if !candidate_type.file_path.starts_with("ext:")
                || candidate_type.qualified_name == *qname
            {
                continue;
            }
            let ext_candidate =
                format!("{}.{seg_name}", candidate_type.qualified_name);
            if let Some(hit) = self.lookup.by_qualified_name(&ext_candidate) {
                return Some(hit.clone());
            }
        }
        // Namespace-qualifier fallback. C# `using Foo.Bar;` brings every
        // top-level type from `Foo.Bar.*` into scope by short name. When a
        // chain receiver like `JsonConvert` resolves to a short qname, the
        // member `JsonConvert.SerializeObject` lives under a namespaced
        // qname `Newtonsoft.Json.JsonConvert.SerializeObject`. Try each
        // wildcard import as a prefix on the current qname so the engine
        // matches the legacy walker's using-directive resolution path.
        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            let Some(module) = import.module_path.as_deref() else {
                continue;
            };
            let ns_candidate = format!("{module}.{qname}.{seg_name}");
            if let Some(hit) = self.lookup.by_qualified_name(&ns_candidate) {
                return Some(hit.clone());
            }
        }
        // Inheritance walk via parent_class_qname. When the member isn't a
        // direct child of current_ty's qname, climb the extends chain and
        // retry the lookup at each ancestor. Covers `class UserRepo extends
        // BaseRepo { ... }` where the chain hits an inherited member.
        // MembersIndex.lookup already walks supertypes for Class TypeIds in
        // its direct/extension maps, but the qname-keyed fallback path
        // doesn't see that traversal — externals filtered from the
        // engine member set still need their inherited members found via
        // string-keyed by_qualified_name. Capped at 10 ancestors to guard
        // malformed cycles.
        let mut ancestor: String = qname.clone();
        for _ in 0..10 {
            let parent = match self.lookup.parent_class_qname(&ancestor) {
                Some(p) => p.to_string(),
                None => break,
            };
            let inh_candidate = format!("{parent}.{seg_name}");
            if let Some(hit) = self.lookup.by_qualified_name(&inh_candidate) {
                return Some(hit.clone());
            }
            if parent == ancestor {
                break;
            }
            ancestor = parent;
        }
        None
    }

    /// Walk `current_ty` through alias expansion until a non-alias head is
    /// reached or the expander returns `None`. Bounded by the alias
    /// expander's own depth cap.
    fn expand_aliases(&self, current_ty: TypeId) -> TypeId {
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
    fn bind_apply_args(&self, ty: TypeId, env: &mut GenericEnv) {
        let (base, args) = match self.arena.get(ty) {
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

// ---------------------------------------------------------------------------
// Discovery helpers — building blocks for per-language RootResolvers that
// need to identify a framework's implicit receiver type without hardcoding
// version-specific qnames.
// ---------------------------------------------------------------------------

/// Find the qualified name of a type by its canonical member set.
///
/// Given a `seed_method` (a method name that's expected on every variant
/// of the type — e.g. `$emit` for Vue's component instance, `$patch` for
/// Pinia stores, `timeout` for Mocha's test context) and a list of
/// `canonical_members` (sibling methods that must coexist on the same
/// type to disambiguate from impostors), return the qname of the first
/// type in the symbol index whose member set contains all of them.
///
/// This is the canonical mechanism a `RootResolver` implementation
/// should use to discover a framework's receiver type. It works
/// regardless of the framework version, the package layout, or the
/// declaring interface's name — only the structural member shape
/// matters. A new major version that renames the type but keeps the
/// API is automatically picked up; an unrelated library that happens
/// to ship one same-named method is rejected by the sibling check.
///
/// Returns `None` when no candidate satisfies the full canonical set —
/// the project doesn't have the framework installed, or its version
/// is so old/new it lacks the canonical signature.
///
/// Examples:
///
/// - Vue component instance: `discover_type_by_canonical_members(
///   lookup, "$emit", &["$nextTick", "$forceUpdate"])` returns
///   `vue.Vue` on Vue 2, `@vue/runtime-core.ComponentPublicInstance`
///   on Vue 3 (when its members are extracted as interface members),
///   and whatever future Vue names its instance type.
/// - Pinia store: `discover_type_by_canonical_members(lookup, "$patch",
///   &["$reset", "$subscribe", "$dispose"])` returns `pinia.Store` or
///   whichever interface Pinia's d.ts ships.
/// - Mocha context: `discover_type_by_canonical_members(lookup,
///   "timeout", &["skip", "retries", "slow"])` returns
///   `mocha.Context` / `Mocha.Context`.
pub fn discover_type_by_canonical_members(
    lookup: &dyn crate::indexer::resolve::engine::SymbolLookup,
    seed_method: &str,
    canonical_members: &[&str],
) -> Option<String> {
    for sym in lookup.by_name(seed_method) {
        if sym.kind != "method" {
            continue;
        }
        let Some(parent) = sym.qualified_name.rsplit_once('.').map(|(p, _)| p) else {
            continue;
        };
        if parent.is_empty() {
            continue;
        }
        let all_present = canonical_members.iter().all(|member| {
            let qname = format!("{parent}.{member}");
            lookup.by_qualified_name(&qname).is_some()
        });
        if all_present {
            return Some(parent.to_string());
        }
    }
    None
}

/// True when `name` is composed only of characters that may appear in a
/// legitimate type identifier across the languages this resolver serves
/// (JS/TS/Vue/Python/Java/C#/Rust/Go/Ruby/PHP/Scala/Kotlin/Swift/Ada/...).
/// All accept `[A-Za-z0-9_]` and JS/TS additionally accepts `$`.
///
/// Used by the SelfRef fallback to reject `SymbolKind::Class` rows whose
/// `name` is actually a CSS class selector (`editor-slide-upload`,
/// `vue-image-crop-upload`, multi-line descendant chains). Those leak in
/// from the SCSS extractor inside Vue/Svelte/Astro `<style>` blocks and
/// would otherwise be treated as the file's primary type.
fn is_identifier_like(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// File stem (basename without extension) of `file_path`. Returns `None`
/// for empty / extension-less paths so the caller falls back to the
/// single-candidate rule. Path may use either separator and may be a
/// virtual `ext:...` URI — splitting on both `/` and `\` and stripping
/// the trailing extension handles every shape the indexer produces.
fn file_stem_of(file_path: &str) -> Option<String> {
    let basename = file_path
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(file_path);
    if basename.is_empty() {
        return None;
    }
    let stem = match basename.rfind('.') {
        Some(dot) if dot > 0 => &basename[..dot],
        _ => basename,
    };
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
