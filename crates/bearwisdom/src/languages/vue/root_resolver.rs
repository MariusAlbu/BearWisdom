// =============================================================================
// languages/vue/root_resolver.rs — Vue chain-walker RootResolver
//
// Owns the `this` (SelfRef) root-type resolution for Vue SFCs. `this`
// inside an Options-API method body is the framework's component
// instance type, declared in the project's installed Vue package — the
// extractor cannot capture this from source code because no user-source
// `extends` clause is present.
//
// Discovery is *structural*, not version-keyed: we ask the symbol
// index for any externally-walked interface whose member set contains
// the canonical Vue instance API (`$emit`, `$nextTick`, `$forceUpdate`)
// and pick the one with the most matches. That works for Vue 2's
// `vue.Vue` (in `vue/types/vue.d.ts`), Vue 3's
// `@vue/runtime-core.ComponentPublicInstance` (in
// `@vue/runtime-core/dist/runtime-core.d.ts`), and any future version
// that ships the same instance API under a different name — no per-
// version branching in code.
//
// Non-SelfRef segments delegate to `DefaultRootResolver`; the Vue
// resolver is *purely* an override for the receiver-type question, not
// a parallel chain walker.
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::core::chain::{DefaultRootResolver, RootResolver};
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::{ChainSegment, SegmentKind};

pub struct VueRootResolver;

impl RootResolver for VueRootResolver {
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
                // First honour any explicit scope_path the extractor
                // attached (Vue + TS Options API with `defineComponent`
                // produces methods scoped under the SFC class). When the
                // source symbol has a real enclosing type, prefer it.
                if let Some(scope) = ref_ctx.source_symbol.scope_path.as_ref() {
                    if !scope.is_empty() {
                        return Some(arena.class(scope));
                    }
                }
                // No enclosing scope — discover the project's Vue
                // component instance type from the externally-walked
                // d.ts and use it as the receiver. `this.$emit`,
                // `this.$nextTick`, `this.$store.dispatch`, etc. then
                // resolve as ordinary member walks against the
                // discovered type.
                if let Some(qname) = discover_component_instance(lookup) {
                    return Some(arena.class(&qname));
                }
                // Fallback: defer to the default resolver's own
                // SelfRef behaviour (file-primary-class). Keeps SFCs
                // with no recognisable framework install still
                // resolving against their own class symbol.
                DefaultRootResolver.resolve(seg, ref_ctx, file_ctx, arena, lookup)
            }
            _ => DefaultRootResolver.resolve(seg, ref_ctx, file_ctx, arena, lookup),
        }
    }
}

pub static VUE_ROOT_RESOLVER: VueRootResolver = VueRootResolver;

/// Canonical members of a Vue component instance interface. A type that
/// declares ALL three is the framework's component instance type for the
/// project, regardless of which package or version exports it. The set
/// is small enough that the discovery cost is two extra qname probes
/// per `$emit`-named symbol — typically one or two index entries.
const VUE_INSTANCE_CANONICAL_METHODS: &[&str] = &["$nextTick", "$forceUpdate"];

/// Find the qualified name of the project's Vue component instance type.
///
/// Strategy:
///   1. Enumerate every symbol named `$emit` in the index.
///   2. For each, strip the trailing `.$emit` to recover the parent
///      qname (the candidate instance interface).
///   3. Require the parent to also declare `$nextTick` AND
///      `$forceUpdate` — the canonical Vue instance signature shared
///      by every major version.
///   4. Return the first parent that satisfies the check. (Both Vue 2
///      and Vue 3 declare exactly one interface that does — the
///      framework's primary instance type. Other libraries that happen
///      to declare a method named `$emit` won't also declare the rest
///      of the set.)
///
/// Returns `None` when no `$emit`-bearing interface in the index
/// matches — the project has no Vue install, or the install is too old
/// / too new to expose the canonical signature.
pub(crate) fn discover_component_instance(lookup: &dyn SymbolLookup) -> Option<String> {
    for sym in lookup.by_name("$emit") {
        if sym.kind != "method" {
            continue;
        }
        let Some(parent) = sym.qualified_name.rsplit_once('.').map(|(p, _)| p) else {
            continue;
        };
        if parent.is_empty() {
            continue;
        }
        let all_present = VUE_INSTANCE_CANONICAL_METHODS.iter().all(|member| {
            let qname = format!("{parent}.{member}");
            lookup.by_qualified_name(&qname).is_some()
        });
        if all_present {
            return Some(parent.to_string());
        }
    }
    None
}

#[cfg(test)]
#[path = "root_resolver_tests.rs"]
mod tests;
