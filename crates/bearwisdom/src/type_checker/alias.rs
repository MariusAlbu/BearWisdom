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

use crate::indexer::resolve::engine::SymbolLookup;
use crate::type_checker::type_env::TypeEnvironment;
use crate::types::AliasTarget;

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
                    .field_type_name(value_name)
                    .or_else(|| lookup.return_type_name(value_name))
                    .map(|s| s.to_string());
                let Some(new_head) = resolved else {
                    // Value not indexed or has no recorded type — leave
                    // the chain walker with what it had before so it can
                    // record a proper miss against the alias name.
                    return None;
                };
                (new_head, Vec::new())
            }
            // Non-application shapes have no single "head" to follow.
            // Future PRs add their own machinery (member-set semantics for
            // Union/Intersection, mapped/conditional expanders). `Keyof`
            // produces a string-literal union — also no head to walk.
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

#[cfg(test)]
#[path = "alias_tests.rs"]
mod tests;
