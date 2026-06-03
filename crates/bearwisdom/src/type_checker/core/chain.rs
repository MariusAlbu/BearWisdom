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
// Known scope limitations:
//   - Optional-chaining (`a?.b`) preserves the chain miss/hit on `b` but the
//     yield type does not re-wrap in Optional. Affects flow-typing of the
//     downstream binding, not target-symbol resolution.
//   - Flow narrowing refines only the ROOT receiver: the default root resolver
//     reads `FlowMeta.narrowings` via `SymbolLookup::local_type` + a position
//     cursor. Mid-chain segments and union-branch selection don't consult it.
//   - Dispatch: members resolve by receiver dispatch (MembersIndex::lookup) for
//     every language. The MultiArg / ReturnType axes in `dispatch::select_method`
//     are configured per-language but not yet consumed by this walker.
// =============================================================================

use super::types::{GenericParamId, Type, TypeArena, TypeId};
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolInfo, SymbolLookup};
use crate::type_checker::alias::{expand_alias_typed, AliasIndex};
use crate::type_checker::core::generics::{substitute, unify_into, GenericEnv};
use rustc_hash::{FxHashMap, FxHashSet};
use crate::type_checker::core::dispatch::{
    arg_assignable_candidates, resolve_arg_types, select_method, DispatchQuery,
};
use crate::type_checker::core::members::{ArgTypes, MembersIndex};
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::core::symbol_view::SymbolView;
use crate::type_checker::profile::language_profile::{DispatchAxis, LanguageProfile};
use crate::types::{CallArg, ChainSegment, EdgeKind, MemberChain, SegmentKind};

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
                //    of the same name. `local_type_union` consults the
                //    per-file LocalTypeCache that the resolver loop populates
                //    as it encounters assignments and CFG narrowings; the
                //    cursor is moved before each ref so narrowings honour the
                //    current position. A multi-branch fact (`if (typeof x ===
                //    "string" || typeof x === "number")`) builds a real
                //    `Type::Union` here; downstream member lookup
                //    (`core/members.rs` Union arm) requires every branch to
                //    carry the member, which is the safe semantic — a runtime
                //    value could land on any branch.
                if let Some(branches) = lookup.local_type_union(&seg.name) {
                    return Some(match branches.as_slice() {
                        [one] => arena.class(one),
                        many => {
                            let ids: Vec<TypeId> =
                                many.iter().map(|n| arena.class(n)).collect();
                            arena.intern(Type::Union(ids))
                        }
                    });
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
        // A root segment's `declared_type` is a type assertion: a `(x as Foo)`
        // cast, or a synthetic-root type (`[]` → Array, `import()` → Promise).
        // It wins over scope-based root resolution — adopt the asserted type.
        let mut current_ty = match root_seg.declared_type.as_deref().filter(|s| !s.is_empty()) {
            Some(dt) => self.arena.intern_type_str(dt),
            None => root.resolve(root_seg, ref_ctx, file_ctx, self.arena, self.lookup)?,
        };
        current_ty = self.expand_aliases(current_ty);
        current_ty = self.narrow_union_by_discriminant(current_ty, root_seg);

        // Root-call form: `f().x` where the root `f` is a function-typed value.
        // Peel the invoked function to its return type before walking members
        // (mirrors `unwrap_if_called` for mid-chain function-typed members).
        if chain.segments.len() > 1 && root_seg.is_call {
            if let Type::Function { return_, .. } = self.arena.get(current_ty) {
                current_ty = self.expand_aliases(return_);
            }
        }

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

            // Cast / type-assertion mid-chain (`(a.x() as Foo).y`): the
            // `declared_type` asserts the value's type — adopt it and skip
            // member resolution on this segment.
            if let Some(dt) = seg.declared_type.as_deref().filter(|s| !s.is_empty()) {
                current_ty = self.expand_aliases(self.arena.intern_type_str(dt));
                self.bind_apply_args(current_ty, &mut env);
                continue;
            }

            let kind_filter = if i == last_idx {
                ref_ctx.extracted_ref.kind
            } else {
                EdgeKind::TypeRef
            };

            // Overload disambiguation. On the final call segment the resolved
            // ref carries the arguments: their count drives arity selection and
            // their resolved types drive type-based overload selection. An empty
            // list is ambiguous (zero args vs. not extracted), so it leaves both
            // off and lookup keeps first-match behavior.
            let is_call_seg = i == last_idx
                && matches!(kind_filter, EdgeKind::Calls | EdgeKind::Instantiates)
                && !ref_ctx.extracted_ref.call_args.is_empty();
            let arg_count = is_call_seg.then(|| ref_ctx.extracted_ref.call_args.len());
            let arg_type_ids: Vec<TypeId> = if is_call_seg {
                resolve_arg_types(
                    &ref_ctx.extracted_ref.call_args,
                    self.arena,
                    self.lookup,
                    self.profile,
                )
            } else {
                Vec::new()
            };
            let arg_types = is_call_seg.then_some(ArgTypes {
                arg_types: &arg_type_ids,
                symbol_types: self.symbol_types,
                lookup: self.lookup,
            });

            // Non-receiver dispatch axes (multi-arg / return-type) route through
            // the dispatch front door; the receiver axis stays on the direct
            // member lookup so the common path is unchanged.
            let resolved = if is_call_seg
                && !matches!(self.profile.dispatch_axis, DispatchAxis::Receiver)
            {
                let query = DispatchQuery {
                    method_name: &seg.name,
                    receiver: current_ty,
                    arg_types: &arg_type_ids,
                    expected_return: None,
                    kind_filter,
                };
                select_method(
                    &query,
                    self.members,
                    self.supertypes,
                    self.symbol_types,
                    self.arena,
                    self.profile,
                    self.lookup,
                )
                .map(|m| (m, current_ty, Vec::new()))
            } else {
                self.members.lookup_with_binding(
                    current_ty,
                    &seg.name,
                    kind_filter,
                    self.supertypes,
                    self.arena,
                    self.profile,
                    arg_count,
                    arg_types,
                )
            };

            let member = match resolved {
                Some((m, owner, owner_args)) => {
                    // Inherited generic method: bind the ancestor's generic
                    // params to the arguments named on the `extends` /
                    // `implements` edge (`Repository<User>`), so a member
                    // returning `T` substitutes to the concrete type at the
                    // yield step below.
                    if !owner_args.is_empty() {
                        let params = self.owner_generic_params(owner);
                        if !params.is_empty() {
                            env.bind_positional(&params, &owner_args);
                        }
                    }
                    m
                }
                None => self.qualified_member_lookup(current_ty, &seg.name, file_ctx)?,
            };

            // G1: explicit call-site type arguments (turbofish `m<User>()`) bind
            // the called method's own generic parameters, so a method returning
            // one of them yields the concrete type. The param ids come from
            // `generic_param_type_ids` — the same source `owner_param_type_map`
            // canonicalizes the return to — so the binding and the rebound
            // return agree on the `GenericParamId`.
            if !seg.type_args.is_empty() {
                if let Some(param_tys) = self.lookup.generic_param_type_ids(&member.qualified_name) {
                    let params: Vec<GenericParamId> = param_tys
                        .iter()
                        .filter_map(|&id| match self.arena.get(id) {
                            Type::Generic { param } => Some(param),
                            _ => None,
                        })
                        .collect();
                    if !params.is_empty() {
                        let arg_ids: Vec<TypeId> = seg
                            .type_args
                            .iter()
                            .map(|s| self.arena.intern_type_str(s))
                            .collect();
                        env.bind_positional(&params, &arg_ids);
                    }
                }
            }

            // G2: argument-driven generic inference (INFER-8). With no
            // explicit turbofish, unify the called method's declared parameter
            // types against the resolved argument types to bind the type
            // parameters its return resolves through, so a method returning one
            // of them yields the concrete type. The bindable param set and the
            // canonical `Generic` ids both come from `owner_param_type_map` —
            // the SAME map `yield_type_of` rebinds the return with — so a bound
            // param and the substituted return agree on the `GenericParamId`.
            // Both the method's own params and the owning type's are bindable;
            // `unify_into` binds only an UNBOUND slot, so the receiver binding
            // already set by `bind_apply_args` stays authoritative — an argument
            // can FILL an owner param the receiver left unbound (raw
            // `Repository`) but never override a receiver-pinned one
            // (`Repository<Account>`). The declared param types are interned
            // param-blind (`Class("T")`), so they are rebound to the canonical
            // ids before unifying.
            if is_call_seg && seg.type_args.is_empty() {
                self.bind_call_arg_generics(
                    &member.qualified_name,
                    member.id,
                    &arg_type_ids,
                    &mut env,
                );
            }

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
        let view = SymbolView::new(sym, self.symbol_types);
        let data_raw = match sym.kind.as_str() {
            "method" | "function" | "constructor" => view.return_type(),
            "field" | "property" | "variable" | "parameter" | "enum_member" => {
                view.declared_type()
            }
            "class" | "struct" | "interface" | "trait" | "enum" | "type_alias"
                if seg.kind == SegmentKind::Construction =>
            {
                view.return_type()
            }
            _ => view.declared_type().or(view.return_type()),
        };
        if let Some(raw) = data_raw {
            // Canonicalize nominal param tokens (`Class("T")`/`Class("U")`) in
            // the stored return to the owner's / method's `Generic` ids before
            // substituting. The SymbolTypeMap return is interned param-blind
            // (`intern_type_str` yields `Class`), so without this an inherited
            // or turbofish-bound generic stays unresolved on this path — only
            // the string-fallback path below rebinds. Already-`Generic` returns
            // are untouched (`rebind_class_params` rewrites only `Class`).
            let params = self.owner_param_type_map(&sym.qualified_name);
            let raw = if params.is_empty() {
                raw
            } else {
                self.arena.rebind_class_params(raw, &params)
            };
            return Some(self.unwrap_if_called(substitute(raw, env, self.arena), seg, &sym.kind));
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
        // Rebind nominal param tokens (`Class("T")`) to the owner's canonical
        // `Generic(T)` so a generic return substitutes against the receiver's
        // bound args; `intern_type_str` alone is param-blind.
        let params = self.owner_param_type_map(&sym.qualified_name);
        let yielded = if params.is_empty() {
            yielded
        } else {
            self.arena.rebind_class_params(yielded, &params)
        };
        Some(self.unwrap_if_called(substitute(yielded, env, self.arena), seg, &sym.kind))
    }

    /// When a function-typed value member (field/property/variable/parameter)
    /// is invoked, the chain yields the function's return type rather than the
    /// function value. A method/function member's resolved type is already its
    /// return type, so it is never peeled here.
    fn unwrap_if_called(&self, ty: TypeId, seg: &ChainSegment, sym_kind: &str) -> TypeId {
        if seg.is_call
            && matches!(sym_kind, "field" | "property" | "variable" | "parameter")
        {
            if let Type::Function { return_, .. } = self.arena.get(ty) {
                return return_;
            }
        }
        ty
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
            match expand_alias_typed(
                ty,
                self.arena,
                self.aliases,
                self.lookup,
                self.members,
                self.symbol_types,
            ) {
                Some(next) if next != ty => {
                    ty = next;
                }
                _ => break,
            }
        }
        ty
    }

    /// Narrow a discriminated-union receiver to the branch selected by an active
    /// discriminant guard. When `root_seg` is an identifier with a guard like
    /// `if (x.kind === "circle")` in scope and `ty` is a `Type::Union`, return
    /// the branch whose discriminant property (`kind`) carries that literal
    /// (stored on the property's signature for literal-typed fields). Returns
    /// `ty` unchanged when there's no union, no active guard, or no branch
    /// matches — the union arm then falls back to its first-branch behavior.
    fn narrow_union_by_discriminant(&self, ty: TypeId, root_seg: &ChainSegment) -> TypeId {
        if root_seg.kind != SegmentKind::Identifier {
            return ty;
        }
        // A `Union` is a nominal discriminated union; an `Intersection` is how
        // an anonymous discriminated union (`{kind:"a";…}|{kind:"b";…}`) is
        // represented — its any-branch member lookup preserves flat resolution
        // when no guard is active, and a guard narrows it to one branch here.
        let (branches, is_union) = match self.arena.get(ty) {
            Type::Union(b) => (b, true),
            Type::Intersection(b) => (b, false),
            _ => return ty,
        };
        let Some((prop, literal, negate)) = self.lookup.local_discriminant(&root_seg.name) else {
            return ty;
        };
        let matches_literal = |branch: TypeId| -> bool {
            self.members
                .lookup(
                    branch,
                    &prop,
                    EdgeKind::TypeRef,
                    self.supertypes,
                    self.arena,
                    self.profile,
                )
                .and_then(|m| m.signature)
                .as_deref()
                == Some(literal.as_str())
        };
        if negate {
            // Early-exit guard (`if (x.kind !== "lit") return;`): keep the
            // branches whose discriminant is NOT the literal. One survivor
            // narrows to it; several keep a sub-container (same kind as the
            // original); excluding none or all leaves the receiver unchanged.
            let kept: Vec<TypeId> = branches.iter().copied().filter(|&b| !matches_literal(b)).collect();
            match kept.len() {
                1 => kept[0],
                n if n == 0 || n == branches.len() => ty,
                _ => {
                    let narrowed = if is_union {
                        Type::Union(kept)
                    } else {
                        Type::Intersection(kept)
                    };
                    self.arena.intern(narrowed)
                }
            }
        } else {
            branches.into_iter().find(|&b| matches_literal(b)).unwrap_or(ty)
        }
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
        let params = self.owner_generic_params(base);
        if params.is_empty() {
            return;
        }
        env.bind_positional(&params, &args);
    }

    /// Declared generic params of a type as `GenericParamId`s for positional
    /// binding. Prefers extractor-populated `symbol_types`; falls back to the
    /// build-time canonical `Type::Generic` ids in `generic_param_type_ids` —
    /// the source that is actually populated in a real index.
    fn owner_generic_params(&self, owner_ty: TypeId) -> Vec<GenericParamId> {
        if let Some(data) = self.symbol_types.data_for_class(owner_ty) {
            if !data.generic_params.is_empty() {
                return data.generic_params.clone();
            }
        }
        let qname = match self.arena.get(owner_ty) {
            Type::Class(q) => q,
            _ => return Vec::new(),
        };
        let Some(ids) = self.lookup.generic_param_type_ids(&qname) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|&id| match self.arena.get(id) {
                Type::Generic { param } => Some(param),
                _ => None,
            })
            .collect()
    }

    /// `{param-name → Type::Generic id}` for both the member's own type
    /// parameters (`find<U>`) and its owning type's (`class Repo<T>`). Drives
    /// the rebind of a member's declared/return type from nominal `Class("T")`
    /// to the canonical `Generic(T)` so `substitute` resolves it against the
    /// receiver's bound args and any call-site turbofish. A method param
    /// shadows a same-named owning-type param.
    fn owner_param_type_map(&self, member_qname: &str) -> FxHashMap<String, TypeId> {
        let mut map = FxHashMap::default();
        if let Some(ids) = self.lookup.generic_param_type_ids(member_qname) {
            for &id in ids {
                map.insert(self.arena.format_type(id), id);
            }
        }
        if let Some((owner, _)) = member_qname.rsplit_once('.') {
            if let Some(ids) = self.lookup.generic_param_type_ids(owner) {
                for &id in ids {
                    map.entry(self.arena.format_type(id)).or_insert(id);
                }
            }
        }
        map
    }

    /// Bind a callee's generic type parameters from resolved argument types
    /// (INFER-8). Rebinds the param-blind declared parameter types to the
    /// callee's canonical `Generic` ids (via `owner_param_type_map` — the same
    /// map the return is rebound through), then unifies each against the
    /// matching argument type, filling only UNBOUND bindable slots so a
    /// receiver-pinned owner parameter stays authoritative. Shared by the
    /// in-chain terminal-call branch and the bare-name call site.
    fn bind_call_arg_generics(
        &self,
        member_qname: &str,
        member_id: i64,
        arg_type_ids: &[TypeId],
        env: &mut GenericEnv,
    ) {
        let name_to_id = self.owner_param_type_map(member_qname);
        let bindable: FxHashSet<GenericParamId> = name_to_id
            .values()
            .filter_map(|&id| match self.arena.get(id) {
                Type::Generic { param } => Some(param),
                _ => None,
            })
            .collect();
        if bindable.is_empty() {
            return;
        }
        if let Some(data) = self.symbol_types.get(member_id) {
            for (param_ty, &arg_ty) in data.param_types.iter().zip(arg_type_ids.iter()) {
                let canon = self.arena.rebind_class_params(*param_ty, &name_to_id);
                unify_into(canon, arg_ty, &bindable, env, self.arena);
            }
        }
    }

    /// Infer the yield type of a chain-less (bare-name) call from its arguments
    /// (INFER-8 at the bare-name site). For `const x = genericFn(u)` where
    /// `genericFn<T>(x: T): T`, binds `T` to the resolved type of `u` and
    /// yields the substituted return, so the forward-flow cache types `x`
    /// precisely instead of as the unbindable bare parameter `T`.
    ///
    /// Returns `None` — leaving the resolver loop's signature/return-type
    /// fallback unchanged — when the callee is non-generic, carries no
    /// arguments, has no inferable return, or the bound result is still a bare
    /// generic. It reports only a strict gain over that fallback, never a
    /// regression (the fallback already records the unbound parameter today).
    pub fn infer_bare_call_yield(
        &self,
        sym: &SymbolInfo,
        call_args: &[CallArg],
    ) -> Option<TypeId> {
        if call_args.is_empty()
            || !matches!(sym.kind.as_str(), "method" | "function" | "constructor")
        {
            return None;
        }
        // A non-generic callee binds nothing; its concrete return flows through
        // the loop's return-type fallback, so don't duplicate it here.
        let name_to_id = self.owner_param_type_map(&sym.qualified_name);
        if name_to_id.is_empty() {
            return None;
        }
        // Declared return, param-blind from the SymbolTypeMap (or the string
        // fallback), rebound to the callee's canonical `Generic` ids so
        // substitution can resolve it.
        let raw = SymbolView::new(sym, self.symbol_types)
            .return_type()
            .or_else(|| {
                self.lookup
                    .return_type_name(&sym.qualified_name)
                    .map(|s| self.arena.intern_type_str(s.trim().trim_end_matches('.')))
            })?;
        let raw = self.arena.rebind_class_params(raw, &name_to_id);

        let arg_type_ids =
            resolve_arg_types(call_args, self.arena, self.lookup, self.profile);
        let mut env = GenericEnv::new();
        self.bind_call_arg_generics(&sym.qualified_name, sym.id, &arg_type_ids, &mut env);

        let yielded = substitute(raw, &env, self.arena);
        // Report only a strict improvement: a binding actually rewrote the
        // return, and the result is neither `Unknown` nor a still-unbound
        // `Generic` (both of which are no better than the existing fallback).
        if yielded == raw {
            return None;
        }
        match self.arena.get(yielded) {
            Type::Unknown | Type::Generic { .. } => None,
            _ => Some(yielded),
        }
    }

    /// Disambiguate a chain-less (bare-name) call across the same-name overloads
    /// in the resolved target's own enclosing scope. The overload set is the
    /// callables named `target_name` that share `current`'s file and scope_path;
    /// when the call's arguments uniquely select exactly one of them, return that
    /// one's symbol id.
    ///
    /// Selection is arity-primary: the assignability filter rejects any candidate
    /// whose parameter count differs from the call's, and further rejects on
    /// parameter type when the argument types are concrete (an `Unknown` argument
    /// is assignable to any parameter of the matching arity).
    ///
    /// This OVERRIDES an already-resolved confidence-1.0 edge, so the gate is
    /// strict on two axes:
    /// - SCOPE: candidates are restricted to `current`'s file AND scope_path, so a
    ///   coincidental same-name callable in an unrelated module can never hijack
    ///   the binding. `by_name` is a whole-program map; without this filter a
    ///   homonym in another scope whose arity happens to match could retarget a
    ///   correctly scoped first-match.
    /// - UNIQUENESS: returns `Some(id)` only when EXACTLY ONE candidate survives.
    ///   Zero survivors (no overload accepts these args) or two-or-more (still
    ///   ambiguous) return `None`, leaving the first-match intact.
    /// Most-specific-among-survivors selection is not used here — uniqueness only.
    pub fn select_bare_overload(
        &self,
        target_name: &str,
        call_args: &[CallArg],
        current: &SymbolInfo,
    ) -> Option<i64> {
        if call_args.is_empty() {
            return None;
        }
        // The overload set is the same-name callables sharing `current`'s
        // enclosing scope (same file AND scope_path) — its true overloads, not
        // every whole-program homonym `by_name` returns.
        let candidates: Vec<SymbolInfo> = self
            .lookup
            .by_name(target_name)
            .iter()
            .filter(|s| matches!(s.kind.as_str(), "function" | "method" | "constructor"))
            .filter(|s| s.file_path == current.file_path && s.scope_path == current.scope_path)
            .cloned()
            .collect();
        if candidates.len() < 2 {
            return None;
        }
        let arg_type_ids = resolve_arg_types(call_args, self.arena, self.lookup, self.profile);
        let matches = arg_assignable_candidates(
            candidates,
            &arg_type_ids,
            self.members,
            self.symbol_types,
            self.arena,
            self.lookup,
            self.profile,
        );
        if matches.len() == 1 {
            Some(matches[0].id)
        } else {
            None
        }
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
