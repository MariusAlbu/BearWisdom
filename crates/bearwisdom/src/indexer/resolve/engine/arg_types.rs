// =============================================================================
// engine/arg_types — typing a call's arguments
//
// Turns the extracted argument expressions of a call into canonical TypeIds so
// generic binding has something to unify against. Conservative by construction:
// every shape the engine cannot type soundly yields `Type::Unknown`, which the
// unifier skips, leaving the parameter slot open instead of binding it wrong.
// =============================================================================

use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::type_checker::core::types::{PrimKind, Type, TypeArena, TypeId};
use crate::types::CallArg;

/// Type each argument positionally. Positions the engine cannot type hold
/// `Unknown` so later positions stay aligned with the callee's parameters.
pub(crate) fn resolve_arg_types(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    args: &[CallArg],
) -> Vec<TypeId> {
    args.iter()
        .map(|a| resolve_arg_type(lookup, arena, a))
        .collect()
}

/// The type of one argument expression, `Unknown` when it cannot be typed.
fn resolve_arg_type(lookup: &dyn SymbolLookup, arena: &TypeArena, arg: &CallArg) -> TypeId {
    match arg {
        CallArg::Ident(name) => ident_type(lookup, arena, name),
        CallArg::StringLit(_) | CallArg::TemplateLit(_) => arena.primitive(PrimKind::Str),
        CallArg::Literal(text) => literal_type(arena, text),
        CallArg::ArrayLiteral { elements } => array_literal_type(lookup, arena, elements),
        // `await p` has the awaited type: one async layer off what `p` is.
        CallArg::Await { expr } => {
            let inner = resolve_arg_type(lookup, arena, expr);
            match arena.get(inner) {
                Type::AsyncWrapper(awaited) => awaited,
                _ => inner,
            }
        }
        // Both branches must agree — a ternary of two different types is a
        // union the unifier has no sound single binding for.
        CallArg::Ternary {
            then_branch,
            else_branch,
        } => {
            let t = resolve_arg_type(lookup, arena, then_branch);
            let e = resolve_arg_type(lookup, arena, else_branch);
            if t == e {
                t
            } else {
                arena.intern(Type::Unknown)
            }
        }
        _ => arena.intern(Type::Unknown),
    }
}

/// The type of a bare identifier argument: the file-local forward-inferred
/// binding first (it is the use site's own knowledge), then the declared type
/// of the one symbol that name resolves to. Ambiguous names — several symbols
/// share it — decline: binding a generic parameter from the wrong `user` is
/// worse than leaving it open.
fn ident_type(lookup: &dyn SymbolLookup, arena: &TypeArena, name: &str) -> TypeId {
    if let Some(id) = lookup.local_type_id(name) {
        return id;
    }
    if let Some(ty) = lookup.local_type(name) {
        return arena.intern_type_str(&ty);
    }
    let candidates = lookup.by_name(name);
    let [only] = candidates.iter().collect::<Vec<_>>()[..] else {
        return arena.intern(Type::Unknown);
    };
    // A class name used as a VALUE is its constructor (`typeof C`), never an
    // instance — `inject(TasksService)` passes the class object. The
    // constructor form is what a token-shaped parameter (`Type<T>`) unifies
    // an instance out of.
    if only.kind == "class" {
        let instance = arena.class(&only.qualified_name);
        return arena.intern(Type::Constructor(instance));
    }
    lookup
        .field_type_id_of(only.id)
        .or_else(|| lookup.field_type_id(&only.qualified_name))
        .unwrap_or_else(|| arena.intern(Type::Unknown))
}

/// The primitive a bare literal denotes. Text that parses as a number is one;
/// the two boolean words are one; everything else (null, undefined, a nested
/// literal captured as source text) declines.
fn literal_type(arena: &TypeArena, text: &str) -> TypeId {
    let t = text.trim();
    if t == "true" || t == "false" {
        return arena.primitive(PrimKind::Bool);
    }
    if t.parse::<i64>().is_ok() {
        return arena.primitive(PrimKind::Int);
    }
    if t.parse::<f64>().is_ok() {
        return arena.primitive(PrimKind::Float);
    }
    arena.intern(Type::Unknown)
}

/// An array literal types as the canonical sequence application over its
/// element type — the same `Apply { Array, [E] }` form `intern_type_str` mints
/// for a `T[]` suffix, so `f([user])` unifies against a declared `xs: T[]`.
/// Requires every element to agree and be typed: a mixed or partly-untyped
/// literal has no single element type.
fn array_literal_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    elements: &[CallArg],
) -> TypeId {
    let unknown = arena.intern(Type::Unknown);
    let Some(first) = elements.first() else {
        return unknown;
    };
    let elem = resolve_arg_type(lookup, arena, first);
    if elem == unknown {
        return unknown;
    }
    for e in &elements[1..] {
        if resolve_arg_type(lookup, arena, e) != elem {
            return unknown;
        }
    }
    arena.intern(Type::Apply {
        base: arena.class("Array"),
        args: vec![elem],
    })
}

#[cfg(test)]
#[path = "arg_types_tests.rs"]
mod tests;
