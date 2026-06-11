// =============================================================================
// type_checker/alias.rs — Type-alias expansion for chain walking
//
// PR 9 of the type-checker consolidation: the first piece of type-level
// computation. The chain walker calls `expand_alias` at each step so that a
// `current_type` like `UserMap` (defined as `type UserMap = Map<string, User>`)
// is rewritten to `Map` + `[string, User]` before the field/return lookups
// fire. Without this step the walker stalls on alias names because the
// alias itself has no fields or methods — those live on the alias's target.
//
// Why a structural payload, not a heuristic: TS extracts the alias RHS into
// a flat list of TypeRefs that loses union vs. application shape. The
// `AliasTarget` payload captured at parse time records the RHS shape
// directly so this expander can refuse to expand `Union` / `Intersection` /
// `Other` aliases — which would corrupt resolution if treated as
// applications.
// =============================================================================

use crate::indexer::resolve::engine::{parse_type_head_and_args, SymbolLookup};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::core::types::{LitValue, Type, TypeArena, TypeId};
use crate::type_checker::subtype::{is_assignable_to, is_assignable_to_typed, SubtypeResult};
use crate::type_checker::type_env::TypeEnvironment;
use crate::types::AliasTarget;
use rustc_hash::FxHashMap;

/// Maximum alias-of-alias hops to follow before giving up.
///
/// Caps the worst-case work for pathological alias graphs without blocking
/// the realistic cases. TypeScript itself caps recursive aliases at 50 in
/// the official checker; we use 8 because real codebases rarely chain more
/// than three or four aliases (a domain alias → a vendor alias → the
/// concrete generic), and a low cap keeps the loop's total cost bounded
/// in chain-heavy paths like RxJS / Drizzle / Prisma usage.
const MAX_EXPANSION_DEPTH: u8 = 8;

/// Expand a type-alias name into its concrete head type plus type args.
///
/// Returns `Some((root, args))` when:
///   - `name` is registered as an alias whose `AliasTarget` is `Application`,
///   - or chains of `Application` aliases collapse to one.
///
/// Returns `None` when:
///   - `name` is not an alias,
///   - the alias's target is `Union` / `Intersection` / `Object` / `Other`
///     (chain walking can't dereference these without member-set semantics
///     or specialized machinery — left to future PRs),
///   - the chain exceeds `MAX_EXPANSION_DEPTH`.
///
/// Generic substitution: when an alias is generic
/// (`type Foo<T> = Bar<T, string>`), the caller's `current_args` are bound
/// against the alias's declared params via `env.enter_generic_context` and
/// the target's args are resolved through `env`. This returns
/// `("Bar", [<T's binding>, "string"])`.
///
/// The returned args are always in source order — caller can pass them
/// straight back to `env.enter_generic_context` for the next chain hop.
pub fn expand_alias(
    name: &str,
    current_args: &[String],
    lookup: &dyn SymbolLookup,
    env: &mut TypeEnvironment,
) -> Option<(String, Vec<String>)> {
    let mut head = name.to_string();
    let mut args: Vec<String> = current_args.to_vec();
    let mut hops: u8 = 0;
    let mut last_progress = head.clone();

    loop {
        let target = lookup.alias_target(&head)?;
        let (root, target_args) = match target {
            AliasTarget::Application { root, args: tas } => (root.clone(), tas.clone()),
            // `type X = typeof someValue` — dereference the value
            // reference to its type. Look up the value's declared
            // `field_type` first (covers `const x: T = ...` and class
            // properties), then fall back to its `return_type` (for
            // `typeof someFn` where the alias should resolve to the
            // function's return type). The result becomes the new head
            // and the loop re-enters in case the value's type is itself
            // an alias.
            AliasTarget::Typeof(value_name) => {
                let resolved = lookup
                    .field_type_str(value_name)
                    .or_else(|| lookup.return_type_str(value_name));
                let Some(new_head) = resolved else {
                    // Value not indexed or has no recorded type — leave
                    // the chain walker with what it had before so it can
                    // record a proper miss against the alias name.
                    return None;
                };
                (new_head, Vec::new())
            }
            // `type Foo = T[K]` — extract the type of property K from
            // T. Three cases (in priority order):
            //   1. K is a generic param bound in env (`type Foo<K> = T[K]`
            //      with K bound to "name"): resolve through env, then
            //      treat as a literal key.
            //   2. K is a literal string already (extractor stripped
            //      the quotes): look up `T.K`'s field_type directly.
            //   3. K is something else (`keyof T`, a generic param not
            //      yet bound, etc.): bail with None — no single head.
            AliasTarget::IndexedAccess { object, key } => {
                // Resolve the object through env in case it carries a
                // generic param too (`type Foo<T> = T["x"]`).
                let object = env.resolve(object);
                // Substitute K via env if it's a bound param.
                let key = env.resolve(key);
                // After substitution, if the key still looks like a
                // type-expression rather than a property name (starts
                // with uppercase + carries no whitespace looks heuristic
                // and unreliable), the lookup will simply miss — which
                // is correct.
                let member_qname = format!("{object}.{key}");
                let resolved = lookup.field_type_name(&member_qname).map(|s| s.to_string());
                let Some(new_head) = resolved else {
                    return None;
                };
                (new_head, Vec::new())
            }
            // Two expandable mapped shapes:
            //   1. *Transparent* (`Partial`/`Required`/`Readonly`):
            //      `value_template` is `{source}[{anything}]`. Property
            //      access behaves the same as on the source, so the alias
            //      collapses to the concrete source type bound in the
            //      caller's args.
            //   2. *Record-shaped* (`{ [P in K]: V }`): `value_template` is
            //      a flat type head. Every key projects the same value type
            //      (`Map<K, V>` value-slot semantics), so member access
            //      yields that value type, bound from the caller's args when
            //      the head is a generic param.
            // Custom mappings whose value template carries an operator
            // (function value, union, nested index access) return None — the
            // engine must not guess at their member set.
            AliasTarget::Mapped {
                source,
                value_template,
            } => {
                let params = lookup
                    .generic_params(&head)
                    .map(|p| p.to_vec())
                    .unwrap_or_default();
                if is_transparent_mapped(source, value_template) {
                    // Resolve `source` (the alias's generic param,
                    // typically "T") through the caller's args and the
                    // ambient env. The result is the concrete type the
                    // caller passed for that param.
                    let resolved_source =
                        if let Some(idx) = params.iter().position(|p| p == source) {
                            if idx < args.len() {
                                args[idx].clone()
                            } else {
                                env.resolve(source)
                            }
                        } else {
                            env.resolve(source)
                        };
                    if resolved_source == *source {
                        // Still bound to the param name (e.g. `Partial<T>`
                        // where T is unbound) — nothing to expand into.
                        return None;
                    }
                    (resolved_source, Vec::new())
                } else if let Some(value) = record_value_type(value_template, &params, &args) {
                    // Record-shaped mapping (`{ [P in K]: V }`): member
                    // access yields the flat value type. Re-enter the loop
                    // so a value that is itself an alias collapses further.
                    (value, Vec::new())
                } else {
                    return None;
                }
            }
            // `type Foo<T> = T extends U ? X : Y` — pick a branch
            // when the subtype check decides definitively. Bind `check`
            // to the alias's concrete arg first (so the generic param
            // resolves to what the caller passed), then through env.
            //
            // `infer` capture (`type Elem<T> = T extends Array<infer U> ? U : never`):
            // when the true branch IS the infer var and the bound check
            // type is a known `Apply { extends-head, args }`, yield
            // `args[slot]` — the direct Apply-shape match, never routed
            // through `is_assignable_to`. Any other shape (head mismatch,
            // out-of-range slot, true branch not the var) falls through
            // to the subtype check below. Undecidable cases bail with
            // None — chain walker correctly misses, never guesses.
            AliasTarget::Conditional {
                check,
                extends,
                true_branch,
                false_branch,
                infer_binding,
            } => {
                let resolved_check = bind_to_arg(check, &head, &args, lookup, env);
                // Direct Apply-shape match for an `infer` capture takes
                // priority; a declined or absent binding falls through to the
                // nominal subtype check.
                let yielded = infer_binding.as_ref().and_then(|(var, slot)| {
                    infer_yield(&resolved_check, extends, true_branch, var, *slot)
                });
                let next = if let Some(yielded) = yielded {
                    yielded
                } else {
                    let resolved_extends = env.resolve(extends);
                    match is_assignable_to(&resolved_check, &resolved_extends, lookup) {
                        Some(true) => true_branch.clone(),
                        Some(false) => false_branch.clone(),
                        None => return None,
                    }
                };
                (env.resolve(&next), Vec::new())
            }
            // Non-application shapes have no single "head" to follow.
            // Future PRs add their own machinery (member-set
            // semantics for Union/Intersection, member enumeration
            // for Keyof).
            AliasTarget::Union(_)
            | AliasTarget::Intersection(_)
            | AliasTarget::Keyof(_)
            | AliasTarget::Object
            | AliasTarget::Other => return None,
        };

        // Substitute the alias's own generic params (e.g. `T` in
        // `type Foo<T> = Bar<T, string>`) into the target's args using the
        // caller's concrete args. Names that aren't params fall through to
        // `env.resolve` so outer-scope bindings (e.g. an enclosing class's
        // `T`) still flow through. Doing this without `enter_generic_context`
        // keeps `env` unchanged across the call — the chain walker can rely
        // on its own scope discipline for what it pushes between segments.
        let params = lookup
            .generic_params(&head)
            .map(|p| p.to_vec())
            .unwrap_or_default();
        let resolved_args: Vec<String> = target_args
            .iter()
            .map(|arg| {
                if let Some(idx) = params.iter().position(|p| p == arg) {
                    if idx < args.len() {
                        return args[idx].clone();
                    }
                }
                env.resolve(arg)
            })
            .collect();

        // Self-referential aliases (`type Foo = Foo`) would otherwise loop
        // until MAX_EXPANSION_DEPTH; bail immediately when we'd revisit the
        // same head with no further reduction. The post-condition "did the
        // head change?" is the cheapest fixed-point check.
        if root == head && resolved_args == args {
            return None;
        }

        head = root;
        args = resolved_args;
        hops += 1;
        if hops >= MAX_EXPANSION_DEPTH {
            // Bail out, but return what we have — the partial expansion is
            // still more useful than the original alias name for the chain
            // walker's lookups.
            break;
        }
        // If the new head isn't itself an alias, we're done — the common case.
        if lookup.alias_target(&head).is_none() {
            break;
        }
        // Track progress to avoid pathological no-op loops where the alias
        // table contains a cycle the equality check above didn't catch.
        if head == last_progress {
            break;
        }
        last_progress = head.clone();
    }

    Some((head, args))
}

/// Recognise the *transparent* mapped-type pattern
/// `{ [K in keyof T]: T[K] }` (with optional readonly/optional
/// modifiers, which are stripped by the extractor). The check is
/// purely syntactic: `value_template` must start with `source`,
/// followed by `[`, followed by any text, followed by `]`. The text
/// inside the brackets is the iteration variable name; we don't
/// validate it because the alias-target extractor doesn't capture
/// the variable name and the iteration variable is necessarily
/// fresh per mapped type. False positives (`source[other_thing]`
/// where other_thing isn't K) are extremely rare in real TS code.
fn is_transparent_mapped(source: &str, value_template: &str) -> bool {
    if source.is_empty() || value_template.is_empty() {
        return false;
    }
    let Some(rest) = value_template.strip_prefix(source) else {
        return false;
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix('[') else {
        return false;
    };
    rest.trim_end().ends_with(']')
}

/// Resolve the *value-slot* type of a Record-shaped mapped alias.
///
/// A mapped type whose value template is a single plain type head —
/// `{ [P in K]: V }` (`Record<K, V>`) or `{ [K in keyof X]: V }` — projects
/// the SAME value type onto every key, exactly as `Map<K, V>` projects `V`
/// from its value slot. A member lookup on a receiver of this type therefore
/// yields that value type, so chain walking can continue against the value's
/// members.
///
/// `value_template` must be a single bare identifier so the projection is
/// unambiguous. Anything carrying an operator (`T[K]` index access, `() => U`
/// function value, `A | B` union, generic args) is NOT a flat value slot —
/// either it is the transparent `source[...]` pattern handled separately, or
/// it is a custom mapping the engine must not guess at, so this declines.
///
/// When the value head is one of the alias's own generic params it is bound
/// to the caller's concrete arg at that position; a non-param head (a concrete
/// type written directly in the mapping) passes through unchanged. Returns
/// `None` when the head is not a flat identifier or stays unbound to the param
/// name — the caller then misses against the alias rather than guessing.
fn record_value_type(value_template: &str, params: &[String], args: &[String]) -> Option<String> {
    let head = value_template.trim();
    if head.is_empty() || !is_plain_type_head(head) {
        return None;
    }
    if let Some(idx) = params.iter().position(|p| p == head) {
        // Value head is a generic param — bind it to the caller's arg.
        return args.get(idx).cloned();
    }
    // Concrete value head written directly in the mapping (`{ [P in K]: User }`).
    Some(head.to_string())
}

/// True when `s` is a single bare type identifier: a non-empty run of
/// identifier characters (alphanumeric, `_`, `$`) plus the `.` of a dotted
/// qname, with no operator, bracket, or whitespace. Distinguishes the flat
/// value slot of a Record-shaped mapping from index-access / function /
/// generic-application value templates that must not be projected.
fn is_plain_type_head(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '.')
}

/// Bind `name` to the alias's concrete arg when it is one of the alias's own
/// generic params, otherwise resolve it through `env`. Mirrors the target-arg
/// substitution that runs after the match, applied here to a single name (the
/// conditional's `check`) so the param resolves before the subtype / infer
/// decision fires.
fn bind_to_arg(
    name: &str,
    head: &str,
    args: &[String],
    lookup: &dyn SymbolLookup,
    env: &TypeEnvironment,
) -> String {
    let params = lookup
        .generic_params(head)
        .map(|p| p.to_vec())
        .unwrap_or_default();
    if let Some(idx) = params.iter().position(|p| p == name) {
        if idx < args.len() {
            return args[idx].clone();
        }
    }
    env.resolve(name)
}

/// Yield the type captured by an `infer` binding in a conditional's `extends`
/// clause, or `None` when the direct Apply-shape match does not hold.
///
/// Fires only when: the resolved check type is `Apply { extends_head, args }`
/// (its head equals `extends`), the true branch IS the infer `var`, and `slot`
/// indexes a present arg. Returns `Some(args[slot])`. Every other shape — head
/// mismatch, out-of-range slot, or true branch that is not the var — returns
/// `None` so the caller falls back to the subtype check; the infer arm never
/// routes through `is_assignable_to`.
fn infer_yield(
    resolved_check: &str,
    extends: &str,
    true_branch: &str,
    var: &str,
    slot: usize,
) -> Option<String> {
    if true_branch != var {
        return None;
    }
    let (head, type_args) = parse_type_head_and_args(resolved_check);
    if head != extends {
        return None;
    }
    type_args.get(slot).map(|a| a.to_string())
}

// ---------------------------------------------------------------------------
// TypeId form — Phase 2 of the engine pivot.
//
// The legacy string fn above remains for chain-walker callers that still
// operate on alias qnames. New code working off `TypeArena` calls
// `expand_alias_typed` and walks the resulting TypeId tree directly. The
// TypeId path performs one expansion hop per call — recursive alias-of-alias
// is handled by re-entry from the chain walker, not by an internal loop, so
// generic substitution can interpose between hops at a single, principled
// site (Phase 3's MembersIndex::lookup).
// ---------------------------------------------------------------------------

/// TypeId-keyed alias-target map. Built once per indexing run from
/// `ParsedFile::alias_targets` by `build_alias_index`; consumed by
/// `expand_alias_typed` thereafter.
pub type AliasIndex = FxHashMap<TypeId, AliasTarget>;

/// Project the string-keyed `(alias_qname, AliasTarget)` pairs emitted by
/// extractors into a TypeId-keyed map. Each alias qname is interned as a
/// Class TypeId so subsequent lookups match the same TypeId the chain walker
/// holds.
pub fn build_alias_index(pairs: &[(String, AliasTarget)], arena: &TypeArena) -> AliasIndex {
    let mut idx = AliasIndex::with_capacity_and_hasher(pairs.len(), Default::default());
    for (name, target) in pairs {
        let id = arena.class(name);
        idx.insert(id, target.clone());
    }
    idx
}

/// TypeId form of `expand_alias`. Resolves a single alias hop, producing the
/// alias's concrete head type as a fresh TypeId in `arena`.
///
/// Returns `Some(TypeId)` when:
/// - `Application` → builds `Type::Apply { base, args }` (or returns `base`
///   for the no-arg form so the chain walker doesn't carry an empty Apply
///   through unnecessary intern hops).
/// - `Union` / `Intersection` → builds the corresponding Type variant.
/// - `Typeof(name)` → dereferences the value's `field_type_name` (then
///   `return_type_name`) via `lookup` and interns the result as a Class.
/// - `IndexedAccess { object, key }` → looks up `field_type_name("object.key")`
///   and interns as Class.
/// - `Mapped` → transparent-pattern case returns `arena.class(source)`;
///   Record-shaped case (`{ [P in K]: V }` with a flat value head) returns
///   `arena.class(value)` so member access projects the value type.
/// - `Conditional` → consults `is_assignable_to_typed`; picks the deciding
///   branch and interns as Class. An `infer` capture in the extends clause is
///   not resolved here (no applied args reach this entry) — it declines to the
///   subtype check; the string-form `expand_alias` resolves it instead.
///
/// Returns `None` for `Keyof`, `Object`, `Other`, and any `Typeof` /
/// `IndexedAccess` whose underlying lookup misses — the chain walker treats
/// None as "miss against the alias name" rather than guessing.
pub fn expand_alias_typed(
    alias_ty: TypeId,
    arena: &TypeArena,
    aliases: &AliasIndex,
    lookup: &dyn SymbolLookup,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
) -> Option<TypeId> {
    let target = aliases.get(&alias_ty)?.clone();
    match target {
        AliasTarget::Application { root, args } => {
            let base = arena.class(&root);
            if args.is_empty() {
                return Some(base);
            }
            let arg_ids: Vec<TypeId> = args.iter().map(|a| arena.class(a)).collect();
            Some(arena.intern(Type::Apply {
                base,
                args: arg_ids,
            }))
        }
        AliasTarget::Union(branches) => {
            let ids: Vec<TypeId> = branches.iter().map(|b| arena.class(b)).collect();
            Some(arena.intern(Type::Union(ids)))
        }
        AliasTarget::Intersection(branches) => {
            let ids: Vec<TypeId> = branches.iter().map(|b| arena.class(b)).collect();
            Some(arena.intern(Type::Intersection(ids)))
        }
        AliasTarget::Typeof(value_name) => {
            let resolved = lookup
                .field_type_name(&value_name)
                .or_else(|| lookup.return_type_name(&value_name))
                .map(|s| s.to_string())?;
            Some(arena.class(&resolved))
        }
        AliasTarget::IndexedAccess { object, key } => {
            let member_qname = format!("{object}.{key}");
            let resolved = lookup
                .field_type_name(&member_qname)
                .map(|s| s.to_string())?;
            Some(arena.class(&resolved))
        }
        AliasTarget::Mapped {
            source,
            value_template,
        } => {
            if is_transparent_mapped(&source, &value_template) {
                return Some(arena.class(&source));
            }
            // Record-shaped mapping (`{ [P in K]: V }`): the flat value
            // template is the value-slot type, projected onto every key the
            // same way `Map<K, V>` projects `V`. Intern the value head as a
            // Class so member access on the receiver continues against it;
            // a value head that is a generic param stays a param name here —
            // the chain walker binds it against the receiver's args, exactly
            // as the `Application` arm leaves its args unsubstituted.
            let value = record_value_type(&value_template, &[], &[])?;
            Some(arena.class(&value))
        }
        // `infer_binding` is unused on this path: it is consumed only by the
        // string-form `expand_alias`, where the caller's `current_args` carry
        // the concrete checked type (`Array<User>`) so `var` can bind to its
        // `Apply` arg. This entry receives the alias TypeId alone with no
        // applied args, and `check` is stored head-reduced (`Array<User>` →
        // `"Array"`), so the concrete arg is unreachable here — the infer
        // conditional declines to the subtype check, which returns None.
        AliasTarget::Conditional {
            check,
            extends,
            true_branch,
            false_branch,
            infer_binding: _,
        } => {
            let check_id = arena.class(&check);
            let extends_id = arena.class(&extends);
            match is_assignable_to_typed(check_id, extends_id, arena, lookup, members, symbol_types)
            {
                SubtypeResult::Yes => Some(arena.class(&true_branch)),
                SubtypeResult::No => Some(arena.class(&false_branch)),
                SubtypeResult::Unknown => None,
            }
        }
        AliasTarget::Keyof(target) => {
            // `keyof T` is the union of T's property names as string-literal
            // types. With no known members the alias has nothing to expand to —
            // miss against the name rather than invent an empty union.
            let members = lookup.members_of(&target);
            if members.is_empty() {
                return None;
            }
            let mut seen = std::collections::HashSet::new();
            let lits: Vec<TypeId> = members
                .iter()
                .filter(|m| seen.insert(m.name.clone()))
                .map(|m| arena.intern(Type::Literal(LitValue::Str(m.name.clone()))))
                .collect();
            Some(arena.intern(Type::Union(lits)))
        }
        AliasTarget::Object | AliasTarget::Other => None,
    }
}

#[cfg(test)]
#[path = "alias_tests.rs"]
mod tests;
