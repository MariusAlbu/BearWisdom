// =============================================================================
// type_checker/core/dispatch.rs — method selection across dispatch axes
//
// Single front door for "given a method name and a call shape, pick the
// implementation". Dispatches on `profile.dispatch_axis`:
//
//   - `Receiver`   — classic single dispatch. Delegates straight to
//                    `MembersIndex::lookup` against the receiver type's
//                    supertype graph. Covers Java / C# / TS / Python /
//                    Ruby / Go method dispatch.
//
//   - `MultiArg`   — multi-method dispatch (R S4, Clojure defmulti,
//                    Common Lisp defmethod, Julia). Filters the candidate
//                    set by argument-type compatibility; picks the most
//                    specific match. Phase 4 ships the structural skeleton
//                    + first-match strategy; Phase 7 R/Lisp migrations
//                    sharpen specificity scoring.
//
//   - `ReturnType` — return-type dispatch (Haskell typeclass instances,
//                    Rust trait-method specialization when the caller's
//                    return-position binding pins it). Filters candidates
//                    by `expected_return` compatibility. Phase 4 ships the
//                    skeleton + first-match strategy; Phase 8 Haskell
//                    migration drives the full algorithm.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 4
//       research/architecture/04-implementation-phases.html § Phase 4
// =============================================================================

use super::types::{PrimKind, Type, TypeArena, TypeId};
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::type_checker::core::inference::unwrap_await;
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::core::symbol_view::SymbolView;
use crate::type_checker::profile::language_profile::{
    AccessorSlot, ContainerShape, DispatchAxis, LanguageProfile,
};
use crate::type_checker::subtype::{args_assignable, is_assignable_to_typed_with, SubtypeResult};
use crate::types::{CallArg, EdgeKind};

/// Inputs for a dispatch query — all the information a candidate selector
/// could plausibly need across the three axes.
pub struct DispatchQuery<'a> {
    pub method_name: &'a str,
    pub receiver: TypeId,
    pub arg_types: &'a [TypeId],
    pub expected_return: Option<TypeId>,
    pub kind_filter: EdgeKind,
}

/// Select the method that should service this call. Returns the chosen
/// SymbolInfo, or None when no candidate matches.
pub fn select_method(
    query: &DispatchQuery,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    arena: &TypeArena,
    profile: &LanguageProfile,
    lookup: &dyn SymbolLookup,
) -> Option<SymbolInfo> {
    match profile.dispatch_axis {
        DispatchAxis::Receiver => select_receiver(query, members, supertypes, arena, profile),
        DispatchAxis::MultiArg => select_multi_arg(
            query,
            members,
            supertypes,
            symbol_types,
            arena,
            profile,
            lookup,
        ),
        DispatchAxis::ReturnType => select_return_type(
            query,
            members,
            supertypes,
            symbol_types,
            arena,
            profile,
            lookup,
        ),
    }
}

fn select_receiver(
    query: &DispatchQuery,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    arena: &TypeArena,
    profile: &LanguageProfile,
) -> Option<SymbolInfo> {
    members.lookup(
        query.receiver,
        query.method_name,
        query.kind_filter,
        supertypes,
        arena,
        profile,
    )
}

/// Gather every candidate named `method_name` reachable on the receiver's
/// supertype chain, then keep those whose declared parameter types are
/// assignable from the call's `arg_types`. Pick the first surviving match
/// — a more refined scoring (most-specific) lives in the per-language
/// hook that the R / Lisp migrations will wire.
fn select_multi_arg(
    query: &DispatchQuery,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    arena: &TypeArena,
    profile: &LanguageProfile,
    lookup: &dyn SymbolLookup,
) -> Option<SymbolInfo> {
    let prims = profile.primitive_mapping;
    let candidates: Vec<SymbolInfo> = candidates(query, members, supertypes).collect();
    let mut matches = arg_assignable_candidates(
        candidates,
        query.arg_types,
        members,
        symbol_types,
        arena,
        lookup,
        profile,
    );
    if matches.is_empty() {
        // No argument-type match — fall back to receiver dispatch so the call
        // still resolves to a single-dispatch target rather than missing.
        return select_receiver(query, members, supertypes, arena, profile);
    }
    // Prefer the most specific overload: the candidate whose parameter types are
    // assignable to (subtypes of) every other match's at each position. When no
    // single candidate dominates the set, the first match wins.
    let best = most_specific_index(&matches, symbol_types, members, arena, lookup, prims);
    Some(matches.swap_remove(best))
}

/// Filter `candidates` to those whose declared parameter types are assignable
/// from `arg_types`, preserving input order. Backed by `SymbolView`:
///
///   - a candidate WITH a `SymbolTypeData` record is kept when
///     `args_assignable` accepts its `param_types` against `arg_types`
///     (which already rejects on arity mismatch);
///   - a candidate WITHOUT a record (`param_types() == None`) is kept only
///     when the call carries no arguments to discriminate on.
///
/// Shared by the multi-arg dispatch path and the bare-name overload override
/// so both decide candidate survival the same way.
pub(crate) fn arg_assignable_candidates(
    candidates: Vec<SymbolInfo>,
    arg_types: &[TypeId],
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
) -> Vec<SymbolInfo> {
    let prims = profile.primitive_mapping;
    let mut matches: Vec<SymbolInfo> = Vec::new();
    for candidate in candidates {
        let view = SymbolView::new(&candidate, symbol_types);
        match view.param_types() {
            Some(params) => {
                if args_assignable(
                    params,
                    arg_types,
                    arena,
                    lookup,
                    members,
                    symbol_types,
                    prims,
                ) {
                    matches.push(candidate);
                }
            }
            // No type info → can't compare. Accept only when the call has no
            // arguments to discriminate on.
            None => {
                if arg_types.is_empty() {
                    matches.push(candidate);
                }
            }
        }
    }
    matches
}

/// Index of the most specific candidate — one whose parameter types are
/// assignable to every other candidate's at each position. Returns 0 when no
/// candidate dominates the rest (ambiguous overload set).
fn most_specific_index(
    matches: &[SymbolInfo],
    symbol_types: &SymbolTypeMap,
    members: &MembersIndex,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    prims: &[(&str, PrimKind)],
) -> usize {
    let params = |s: &SymbolInfo| {
        SymbolView::new(s, symbol_types)
            .param_types()
            .map(|p| p.to_vec())
            .unwrap_or_default()
    };
    'outer: for i in 0..matches.len() {
        let pi = params(&matches[i]);
        for (j, mj) in matches.iter().enumerate() {
            if i == j {
                continue;
            }
            let pj = params(mj);
            if pi.len() != pj.len() {
                continue 'outer;
            }
            let dominates = pi.iter().zip(pj.iter()).all(|(a, b)| {
                matches!(
                    is_assignable_to_typed_with(
                        *a,
                        *b,
                        arena,
                        lookup,
                        members,
                        symbol_types,
                        prims
                    ),
                    SubtypeResult::Yes
                )
            });
            if !dominates {
                continue 'outer;
            }
        }
        return i;
    }
    0
}

fn select_return_type(
    query: &DispatchQuery,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    arena: &TypeArena,
    profile: &LanguageProfile,
    lookup: &dyn SymbolLookup,
) -> Option<SymbolInfo> {
    if let Some(expected) = query.expected_return {
        for candidate in candidates(query, members, supertypes) {
            let Some(return_ty) = SymbolView::new(&candidate, symbol_types).return_type() else {
                continue;
            };
            if matches!(
                is_assignable_to_typed_with(
                    return_ty,
                    expected,
                    arena,
                    lookup,
                    members,
                    symbol_types,
                    profile.primitive_mapping
                ),
                SubtypeResult::Yes
            ) {
                return Some(candidate);
            }
        }
    }
    // Fall back to single-dispatch when no expected_return is given or no
    // candidate's return type matches it — surfacing SOME target beats
    // silently missing when the return-type signal is weak or absent.
    select_receiver(query, members, supertypes, arena, profile)
}

/// Walk the receiver's supertype chain and yield every candidate named
/// `method_name` (regardless of kind filter — multi-arg matching applies
/// the kind filter at the parameter check, not the receiver walk).
fn candidates<'a>(
    query: &'a DispatchQuery,
    members: &'a MembersIndex,
    supertypes: &'a SupertypeGraph,
) -> impl Iterator<Item = SymbolInfo> + 'a {
    supertypes.walk_up(query.receiver).flat_map(move |t| {
        members
            .direct_of(t)
            .iter()
            .chain(members.extensions_of(t).iter())
            .filter(move |s| s.name == query.method_name)
            .cloned()
            .collect::<Vec<_>>()
    })
}

/// Resolve a call's argument expressions to their types, for overload
/// disambiguation. Literals mint the matching primitive; a bare identifier is
/// chased to its local declared type; structured expressions (ternary, array,
/// await, spread, index, binary) are typed by recursion under conservative
/// rules; anything the extractor couldn't pin down yields `Unknown` (which the
/// dispatch arms treat as "can't reject"). The resulting types feed
/// `DispatchQuery::arg_types`.
pub(crate) fn resolve_arg_types(
    call_args: &[CallArg],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
) -> Vec<TypeId> {
    call_args
        .iter()
        .map(|a| resolve_arg_type(a, arena, lookup, profile))
        .collect()
}

fn resolve_arg_type(
    arg: &CallArg,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
) -> TypeId {
    match arg {
        // String-shaped literals all type as the string primitive.
        CallArg::StringLit(_) | CallArg::TemplateLit(_) | CallArg::TaggedTemplate { .. } => {
            arena.primitive(PrimKind::Str)
        }
        // Numeric / boolean literals. Other literal text (null/undefined,
        // collection literals) is left Unknown rather than guessed.
        CallArg::Literal(text) => match scalar_literal_kind(text) {
            Some(kind) => arena.primitive(kind),
            None => arena.intern(Type::Unknown),
        },
        // A bare identifier: chase its local declared type when known. The
        // name interns as a nominal `Class` — primitive disjointness against a
        // primitive-typed parameter is decided at compare time, not here.
        CallArg::Ident(name) => match lookup.local_type(name) {
            Some(ty) if !ty.is_empty() => arena.intern_type_str(&ty),
            _ => arena.intern(Type::Unknown),
        },
        // `cond ? then : else` — type both value branches; commit only when both
        // resolve to the SAME non-Unknown type. A divergent or partly-Unknown
        // ternary stays Unknown rather than picking a branch.
        CallArg::Ternary {
            then_branch,
            else_branch,
        } => {
            let t = resolve_arg_type(then_branch, arena, lookup, profile);
            let e = resolve_arg_type(else_branch, arena, lookup, profile);
            if t == e && !is_unknown(t, arena) {
                t
            } else {
                arena.intern(Type::Unknown)
            }
        }
        // `[a, b, ...]` — type each element; an array of exactly one distinct
        // non-Unknown element type `T` interns as `Apply<Array, [T]>`. Empty,
        // heterogeneous, or any-Unknown arrays stay Unknown (no element guess).
        CallArg::ArrayLiteral { elements } => {
            resolve_array_literal(elements, arena, lookup, profile)
        }
        // `await expr` — type `expr`, then peel exactly one known async wrapper.
        // `unwrap_await` peels a structural `AsyncWrapper`; a nominal
        // `Apply<Promise, [T]>` (how a `Promise<T>` local interns) is peeled when
        // the wrapper class is in the profile's `async_wrappers`. When `expr` is
        // NOT a known async wrapper the result is Unknown — never the wrapped or
        // un-awaited type.
        CallArg::Await { expr } => {
            let inner = resolve_arg_type(expr, arena, lookup, profile);
            unwrap_async(inner, arena, profile).unwrap_or_else(|| arena.intern(Type::Unknown))
        }
        // `...expr` — the spread contributes its iterable's element type to
        // dispatch. A sequence collection (`Array`/`ReadonlyArray`/`Set`) or a
        // structural iterator yields its element type; anything else (including
        // `Map`, whose spread is entry tuples) is Unknown.
        CallArg::Spread { expr } => {
            let inner = resolve_arg_type(expr, arena, lookup, profile);
            iterable_element(inner, arena).unwrap_or_else(|| arena.intern(Type::Unknown))
        }
        // `container[index]` — `Array<T>[_] → T`, `Map<K,V>[_] → V`, a tuple
        // indexed by an integer literal → that element, a class indexed by a
        // string-literal key → that field's type. Every other shape (non-literal
        // key on a class/tuple, unknown container) is Unknown.
        CallArg::IndexAccess { container, index } => {
            resolve_index_access(container, index, arena, lookup, profile)
        }
        // `left op right` — operator-directed. Relational/equality operators yield
        // Bool regardless of operands; short-circuit operators (`&& || ??`) join
        // the operand type (same non-Unknown type on both sides) or Unknown;
        // arithmetic and bitwise yield a numeric/integer result only when BOTH
        // operands are numeric/integer; `+` yields Str when both are strings,
        // numeric when both are numeric, Unknown otherwise.
        CallArg::Binary { op, left, right } => {
            let l = resolve_arg_type(left, arena, lookup, profile);
            let r = resolve_arg_type(right, arena, lookup, profile);
            resolve_binary(op, l, r, arena, profile)
        }
        // A lambda passed as an argument has no class type for overload
        // dispatch — its element type flows the other way (the higher-order
        // method's callback signature types the lambda's params), so as a
        // dispatch arg it is Unknown.
        CallArg::ObjectKeys(_) | CallArg::Lambda { .. } | CallArg::Other => {
            arena.intern(Type::Unknown)
        }
    }
}

/// True when `ty` is the engine's `Unknown` bailout.
fn is_unknown(ty: TypeId, arena: &TypeArena) -> bool {
    matches!(arena.get(ty), Type::Unknown)
}

/// Type an array literal under the single-element-type rule. Returns
/// `Apply<Array, [T]>` only when every element types to the same non-Unknown
/// `T`; empty / heterogeneous / any-Unknown arrays return `Unknown`.
fn resolve_array_literal(
    elements: &[CallArg],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
) -> TypeId {
    if elements.is_empty() {
        return arena.intern(Type::Unknown);
    }
    let mut elem: Option<TypeId> = None;
    for e in elements {
        let t = resolve_arg_type(e, arena, lookup, profile);
        if is_unknown(t, arena) {
            return arena.intern(Type::Unknown);
        }
        match elem {
            None => elem = Some(t),
            Some(prev) if prev == t => {}
            Some(_) => return arena.intern(Type::Unknown),
        }
    }
    match elem {
        Some(t) => {
            let base = arena.class("Array");
            arena.intern(Type::Apply {
                base,
                args: vec![t],
            })
        }
        None => arena.intern(Type::Unknown),
    }
}

/// Peel one known async wrapper off `ty`, returning the inner type. Handles
/// both the structural `AsyncWrapper(T)` (via `unwrap_await`) and the nominal
/// `Apply<W, [T, ..]>` where `W` is one of the profile's `async_wrappers` class
/// names. Returns `None` when `ty` is not a recognized async wrapper — the
/// caller then yields Unknown rather than the un-awaited type.
fn unwrap_async(ty: TypeId, arena: &TypeArena, profile: &LanguageProfile) -> Option<TypeId> {
    let peeled = unwrap_await(ty, arena);
    if peeled != ty {
        return Some(peeled);
    }
    if let Type::Apply { base, args } = arena.get(ty) {
        if let Type::Class(name) = arena.get(base) {
            if !args.is_empty() && profile.async_wrappers.iter().any(|w| *w == name) {
                return Some(args[0]);
            }
        }
    }
    None
}

/// Element type of a known iterable. Recognizes the structural `Iterator(T)`
/// and the nominal `Apply<W, [T, ..]>` where `W` is a sequence-shaped collection
/// whose first type argument IS the element type (`Array`, `ReadonlyArray`,
/// `Set`). Returns `None` for any other shape — notably `Map<K, V>`, whose
/// spread yields `[K, V]` entries rather than `args[0]`, so peeling its first
/// arg would be unsound.
fn iterable_element(ty: TypeId, arena: &TypeArena) -> Option<TypeId> {
    match arena.get(ty) {
        Type::Iterator(inner) => Some(inner),
        Type::Apply { base, args } if !args.is_empty() => match arena.get(base) {
            Type::Class(name) if matches!(name.as_str(), "Array" | "ReadonlyArray" | "Set") => {
                Some(args[0])
            }
            _ => None,
        },
        _ => None,
    }
}

/// Type a subscript `container[index]` conservatively. Types the container by
/// recursion, then indexes its resolved type.
fn resolve_index_access(
    container: &CallArg,
    index: &CallArg,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
) -> TypeId {
    let cont_ty = resolve_arg_type(container, arena, lookup, profile);
    index_into(cont_ty, index, arena, lookup)
}

/// Index a resolved container type by a (still-structured) index expression.
/// `Array<T>[_] → T`, `Map<K,V>[_] → V`, a tuple indexed by an integer literal
/// → that element, a class indexed by a string-literal key → that field's
/// declared type. Every other shape is Unknown.
pub(crate) fn index_into(
    cont_ty: TypeId,
    index: &CallArg,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
) -> TypeId {
    let unknown = arena.intern(Type::Unknown);
    match arena.get(cont_ty) {
        // `Array<T>[_] → T`; `Map<K, V>[_] → V`. The container's element/value
        // type is independent of the index value, so the index is not typed.
        Type::Apply { base, args } => match arena.get(base) {
            Type::Class(name) if name == "Map" && args.len() >= 2 => args[1],
            Type::Class(name) if name == "Array" && !args.is_empty() => args[0],
            _ => unknown,
        },
        // A tuple indexed by an INTEGER LITERAL yields that positional element.
        // A non-literal index, or one out of range, is Unknown.
        Type::Tuple(elems) => match integer_literal_value(index) {
            Some(i) if (i as usize) < elems.len() => elems[i as usize],
            _ => unknown,
        },
        // A class indexed by a STRING-LITERAL key yields that field's declared
        // type. The field-type map is keyed by `Class.field` qname.
        Type::Class(class_name) => match string_literal_value(index) {
            Some(key) => {
                let qname = format!("{class_name}.{key}");
                match lookup.field_type_name(&qname) {
                    Some(ft) if !ft.is_empty() => arena.intern_type_str(ft),
                    _ => unknown,
                }
            }
            None => unknown,
        },
        _ => unknown,
    }
}

/// Project a container's element/key/value type from its `Apply` args, given
/// the declared shape and slot of a built-in accessor. `Sequence` matches an
/// `Apply` whose base is `Array` / `ReadonlyArray` / `Set` (element = `args[0]`);
/// `Map` matches an `Apply` whose base is `Map` (key = `args[0]`, value =
/// `args[1]`). Returns `None` when the receiver is not an `Apply` of the
/// declared shape or lacks the indexed arg — the element comes from the
/// receiver's structure, never from a method→type table.
pub(crate) fn project_container_slot(
    cont_ty: TypeId,
    shape: ContainerShape,
    slot: AccessorSlot,
    arena: &TypeArena,
) -> Option<TypeId> {
    let Type::Apply { base, args } = arena.get(cont_ty) else {
        return None;
    };
    let Type::Class(base_name) = arena.get(base) else {
        return None;
    };
    let shape_matches = match shape {
        ContainerShape::Sequence => {
            matches!(base_name.as_str(), "Array" | "ReadonlyArray" | "Set")
        }
        ContainerShape::Map => base_name == "Map",
    };
    if !shape_matches {
        return None;
    }
    let idx = match slot {
        AccessorSlot::Element | AccessorSlot::Key => 0,
        AccessorSlot::Value => 1,
    };
    args.get(idx).copied()
}

/// Integer value of an index expression when it is a plain integer literal.
fn integer_literal_value(arg: &CallArg) -> Option<i64> {
    match arg {
        CallArg::Literal(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// String key of an index expression when it is a plain string literal.
fn string_literal_value(arg: &CallArg) -> Option<&str> {
    match arg {
        CallArg::StringLit(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Type a binary expression `left op right` from the operator and operand types.
///
/// - relational/equality comparisons (`== != === !== < > <= >=`) → `Bool`,
///   regardless of operand types;
/// - short-circuit / nullish operators (`&& || ??`) → the common operand type
///   when both sides resolve to the same non-Unknown type, else Unknown. In
///   strict-boolean languages both operands are Bool so the join is Bool
///   (correct); in operand-returning languages (JS/TS/Python/Lua) the result
///   is one of the operands, so the join is sound and conservative.
/// - arithmetic (`- * / %`) → numeric when both operands are numeric, else
///   Unknown;
/// - `+` → `Str` when both are strings, numeric when both are numeric, Unknown
///   for any mixed pair or when either operand is Unknown;
/// - bitwise (`& | ^ << >>`) → integer when both operands are integers, else
///   Unknown.
fn resolve_binary(
    op: &str,
    left: TypeId,
    right: TypeId,
    arena: &TypeArena,
    profile: &LanguageProfile,
) -> TypeId {
    let unknown = arena.intern(Type::Unknown);
    match op {
        "==" | "!=" | "===" | "!==" | "<" | ">" | "<=" | ">=" => arena.primitive(PrimKind::Bool),
        "&&" | "||" | "??" => {
            // The result is one of the operands, not a guaranteed Bool. Commit
            // to the operand type only when both sides agree on the same
            // non-Unknown type; otherwise stay Unknown.
            if left == right && !is_unknown(left, arena) {
                left
            } else {
                unknown
            }
        }
        "-" | "*" | "/" | "%" | "**" => {
            numeric_result(left, right, arena, profile).unwrap_or(unknown)
        }
        "+" => {
            let lk = prim_kind_of(left, arena, profile);
            let rk = prim_kind_of(right, arena, profile);
            match (lk, rk) {
                (Some(PrimKind::Str), Some(PrimKind::Str)) => arena.primitive(PrimKind::Str),
                _ => numeric_result(left, right, arena, profile).unwrap_or(unknown),
            }
        }
        "&" | "|" | "^" | "<<" | ">>" | ">>>" => {
            let lk = prim_kind_of(left, arena, profile);
            let rk = prim_kind_of(right, arena, profile);
            if matches!(lk, Some(PrimKind::Int)) && matches!(rk, Some(PrimKind::Int)) {
                arena.primitive(PrimKind::Int)
            } else {
                unknown
            }
        }
        _ => unknown,
    }
}

/// Numeric result of an arithmetic op: `Some(Int)` when both operands are
/// integers, `Some(Float)` when both are numeric and at least one is a float,
/// `None` when either operand is non-numeric or Unknown.
fn numeric_result(
    left: TypeId,
    right: TypeId,
    arena: &TypeArena,
    profile: &LanguageProfile,
) -> Option<TypeId> {
    let lk = prim_kind_of(left, arena, profile)?;
    let rk = prim_kind_of(right, arena, profile)?;
    match (lk, rk) {
        (PrimKind::Int, PrimKind::Int) => Some(arena.primitive(PrimKind::Int)),
        (PrimKind::Int | PrimKind::Float, PrimKind::Int | PrimKind::Float) => {
            Some(arena.primitive(PrimKind::Float))
        }
        _ => None,
    }
}

/// Resolve a TypeId to its primitive kind, bridging the two ways a primitive
/// can be interned: a structural `Primitive(k)` (from a scalar literal) yields
/// `k` directly; a nominal `Class(q)` (from a local type string) yields `k`
/// only when `q` appears in the profile's `primitive_mapping`. Mirrors
/// `subtype::prim_kind_of` so numeric/string detection agrees across the two.
fn prim_kind_of(ty: TypeId, arena: &TypeArena, profile: &LanguageProfile) -> Option<PrimKind> {
    match arena.get(ty) {
        Type::Primitive(k) => Some(k),
        Type::Class(q) => profile
            .primitive_mapping
            .iter()
            .find(|(n, _)| *n == q.as_str())
            .map(|(_, k)| *k),
        _ => None,
    }
}

/// Classify a literal's source text into a primitive kind. Returns None for
/// shapes the engine does not type as a scalar (null/undefined, collection
/// literals), leaving the argument Unknown.
fn scalar_literal_kind(text: &str) -> Option<PrimKind> {
    let t = text.trim();
    match t {
        "true" | "false" => Some(PrimKind::Bool),
        _ if t.parse::<i64>().is_ok() => Some(PrimKind::Int),
        _ if t.parse::<f64>().is_ok() => Some(PrimKind::Float),
        _ => None,
    }
}

/// Test-only access to the type-directed index step against a pre-resolved
/// container type. Lets a test exercise the tuple / class-key arms with a
/// hand-built `Type::Tuple`, which `intern_type_str` cannot produce from a
/// string local.
#[cfg(test)]
pub(super) fn _test_index_into(
    cont_ty: TypeId,
    index: &CallArg,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
) -> TypeId {
    index_into(cont_ty, index, arena, lookup)
}

/// Test-only access to the async-wrapper peel against a pre-built type.
/// Lets a test exercise the structural `AsyncWrapper(T)` path, which
/// `intern_type_str` cannot mint from a string local. Returns the inner type
/// when `ty` is a recognized async wrapper, else `ty` unchanged.
#[cfg(test)]
pub(super) fn _test_unwrap_async(ty: TypeId, arena: &TypeArena) -> TypeId {
    use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
    unwrap_async(ty, arena, &DEFAULT_PROFILE).unwrap_or(ty)
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
