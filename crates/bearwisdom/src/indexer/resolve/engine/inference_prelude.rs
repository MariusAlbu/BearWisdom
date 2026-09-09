// =============================================================================
// engine/inference_prelude — the pre-sweep type-inference passes
//
// The four initializer/wrapper inference passes every resolve entry point runs
// against a freshly built tree, in dependency order, each under a phase-timer
// scope so a slow first index localizes to the pass responsible.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::indexer::phase_timer;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::ParsedFile;

use super::compilation::Compilation;

/// Run the inference prelude over `tree`. Ordering is load-bearing: wrapper
/// returns must be concrete before forward inference reads them, and the
/// single-init field pass must run before the chain-init pass so a fluent
/// chain roots on the just-typed base binding.
pub(super) fn run(
    tree: &mut Compilation,
    parsed: &[ParsedFile],
    ids: &crate::indexer::symbol_ids::SymbolIds,
    profiles: &FxHashMap<&'static str, &'static LanguageProfile>,
) {
    // Resolve `ReturnType<typeof fn>` declared return types now that the wrapped
    // (possibly external) functions are materialized, so a wrapper's return type
    // is concrete before forward inference reads it.
    {
        let _t = phase_timer::scope("resolve.wrapper_returns");
        tree.resolve_wrapper_return_types(parsed);
    }
    // Infer a wrapper hook's return from `return <call>` — `function usePost() {
    // return useQuery(...) }` makes usePost's return useQuery's, so a
    // `const { data } = usePost()` destructure roots on the result type. Runs
    // after externals materialize so a wrapper of an external call resolves too.
    {
        let _t = phase_timer::scope("resolve.call_wrapper_returns");
        tree.infer_call_wrapper_returns(parsed, ids);
    }
    // Type class fields from their call/new initializer — `m = injectMutation(...)`,
    // `#http = inject(HttpClient)` — so `this.m.mutate()` / `this.#http.get()` root.
    {
        let _t = phase_timer::scope("resolve.field_init_types");
        tree.infer_field_init_types(parsed, profiles, ids);
    }
    // Chain-initialized bindings (`const c = base.with(x).use(cb)`) walk their
    // initializer chain with the full member walker; runs after the single-init
    // pass so a fluent chain roots on the just-typed base binding.
    {
        let _t = phase_timer::scope("resolve.chain_init_types");
        tree.infer_chain_init_types(parsed, profiles, ids);
    }
}
