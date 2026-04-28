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
            // `type Partial<T> = { [K in keyof T]?: T[K] }` and the
            // sibling utility types (Required, Readonly) are
            // *transparent* — accessing a property on the mapped
            // type behaves the same as accessing it on the source.
            // Detect the pattern syntactically: `value_template`
            // matches `{source}[{anything}]`. When it does, the
            // mapped alias collapses to the concrete source type
            // bound in the caller's args; chain walking continues
            // against the source's members directly. Everything
            // else (Record's flat value, fully custom mappings)
            // returns None — those need member synthesis a future
            // PR provides.
            AliasTarget::Mapped {
                source,
                value_template,
            } => {
                if !is_transparent_mapped(source, value_template) {
                    return None;
                }
                // Resolve `source` (the alias's generic param,
                // typically "T") through the caller's args and the
                // ambient env. The result is the concrete type the
                // caller passed for that param.
                let params = lookup
                    .generic_params(&head)
                    .map(|p| p.to_vec())
                    .unwrap_or_default();
                let resolved_source = if let Some(idx) = params.iter().position(|p| p == source) {
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
            }
            // Non-application shapes have no single "head" to follow.
            // Future PRs add their own machinery (member-set
            // semantics for Union/Intersection, subtype-check-driven
            // branch selection for Conditional, member enumeration
            // for Keyof).
            AliasTarget::Union(_)
            | AliasTarget::Intersection(_)
            | AliasTarget::Keyof(_)
            | AliasTarget::Conditional { .. }
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

#[cfg(test)]
#[path = "alias_tests.rs"]
mod tests;
