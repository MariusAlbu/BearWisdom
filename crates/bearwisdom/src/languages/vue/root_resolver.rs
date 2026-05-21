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
use crate::type_checker::core::chain::{
    discover_type_by_canonical_members, DefaultRootResolver, RootResolver,
};
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

/// Find the qualified name of the project's Vue component instance type.
///
/// Thin wrapper around `discover_type_by_canonical_members` that pins
/// Vue's canonical instance signature: `$emit` (seed) plus `$nextTick`
/// and `$forceUpdate` (siblings). Every Vue version since 2.x ships
/// these three on the component instance type, so the discovery picks
/// up `vue.Vue` (Vue 2) and would pick up a future renamed instance
/// interface without any code change here.
///
/// Returns `None` when the project has no Vue install or the install
/// doesn't expose the canonical methods as interface members (Vue 3's
/// `ComponentPublicInstance` is declared as a structural type alias,
/// so its members aren't currently indexed as distinct symbols — that
/// gap is an extractor concern, not a discovery concern).
pub(crate) fn discover_component_instance(lookup: &dyn SymbolLookup) -> Option<String> {
    discover_type_by_canonical_members(lookup, "$emit", &["$nextTick", "$forceUpdate"])
}

#[cfg(test)]
#[path = "root_resolver_tests.rs"]
mod tests;
