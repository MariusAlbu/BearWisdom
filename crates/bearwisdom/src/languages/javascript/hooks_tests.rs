// =============================================================================
// languages/javascript/hooks_tests.rs — engine-hook registration regression
//
// JS and TS share the engine surface (imports, scope chain, qnames, lib
// globals). The JS plugin therefore registers the same hook instance as
// the TS plugin — `TYPESCRIPT_HOOKS`. Tests here pin that wiring so a
// future refactor can't quietly drop it; without these hooks, embedded-JS
// refs in Vue 2 SFCs (and refs from plain .js files) bypass the
// resolver entirely and land in `unresolved_refs`.
// =============================================================================

use crate::languages::LanguagePlugin;
use crate::languages::javascript::JavascriptPlugin;
use crate::languages::typescript::TYPESCRIPT_HOOKS;

#[test]
fn javascript_plugin_exposes_engine_hooks() {
    let plugin = JavascriptPlugin;
    assert!(
        plugin.language_hooks().is_some(),
        "JS plugin must register engine hooks so embedded-JS refs in Vue 2 SFCs \
         and plain .js files reach the TS lib-globals / npm-globals fallbacks"
    );
}

#[test]
fn javascript_plugin_shares_the_typescript_hook_instance() {
    let plugin = JavascriptPlugin;
    let js_hooks = plugin.language_hooks().expect("JS plugin registers hooks");
    // Same instance — JS resolution semantics are identical to TS for
    // everything content-driven (imports, scope chain, qnames, lib globals).
    // Two registrations of the same static pinpoint that JS and TS share
    // the engine surface, with separate plugins only for parsing/extraction
    // (different tree-sitter grammars + node kinds).
    let ts_hooks: &dyn crate::type_checker::profile::hooks::LanguageEngineHooks =
        &TYPESCRIPT_HOOKS;
    let js_ptr = js_hooks as *const _ as *const ();
    let ts_ptr = ts_hooks as *const _ as *const ();
    assert_eq!(
        js_ptr, ts_ptr,
        "JS plugin must reuse TYPESCRIPT_HOOKS — duplicating the hook with a \
         JS-named adapter would split the resolver into two parallel definitions"
    );
}
