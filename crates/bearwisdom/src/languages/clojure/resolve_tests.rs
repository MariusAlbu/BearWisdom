use super::hooks::{
    detect_clj_compojure_route, detect_clj_http_producer, detect_clj_jdbc_db_query,
};
use crate::types::*;

#[test]
fn test_clj_compojure_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_clj_compojure_route("GET", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_clj_compojure_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_clj_compojure_route("POST", &args).is_some());
}

#[test]
fn test_clj_compojure_rejects_lowercase() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_clj_compojure_route("get", &args).is_none());
}

#[test]
fn test_clj_http_get_emits_producer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_clj_http_producer("clj-http.client", "get", &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_clj_http_rejects_non_http_ns() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_clj_http_producer("clojure.core", "get", &args).is_none());
}

#[test]
fn test_clj_jdbc_execute_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_clj_jdbc_db_query("next.jdbc", "execute!").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_clj_jdbc_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_clj_jdbc_db_query("next.jdbc.sql", "insert!").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

// ---------------------------------------------------------------------------
// `:refer`-injected bare-name binding, end-to-end through the generic engine.
//
// `(ns f (:require [clojure.test :refer [is]]))` brings the bare name `is`
// into scope, sourced from `clojure.test`. A bare `(is ...)` call must bind to
// the `clojure.test` namespace's `is` — never to a same-named function defined
// in an unrelated namespace, and never to a local `is` in a file that lacks the
// `:refer`. The fix is `build_file_context` modelling the `:refer` entry as an
// ordinary import binding (`imported_name = is`, `module_path = clojure.test`)
// so the bare-name import rung resolves it.
// ---------------------------------------------------------------------------

use super::extract::extract;
use super::hooks::ClojureHooks;
use crate::indexer::resolve::engine::{
    build_scope_chain, FileContext, RefContext, Resolution, SymbolIndex,
};
use crate::type_checker::core::SymbolIdMap;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::type_checker::Engine;
use std::collections::HashMap;

fn clj_parsed_file(path: &str, src: &str) -> ParsedFile {
    let ex = extract(src);
    ParsedFile {
        path: path.to_string(),
        language: "clojure".to_string(),
        content_hash: String::new(),
        size: src.len() as u64,
        line_count: src.lines().count() as u32,
        mtime: None,
        package_id: None,
        content: Some(src.to_string()),
        has_errors: ex.has_errors,
        symbols: ex.symbols,
        refs: ex.refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

/// Assign one stable id per (path, qname) the same way the engine harness does.
fn id_map_for(files: &[ParsedFile]) -> HashMap<(String, String), i64> {
    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    let mut next = 1i64;
    for pf in files {
        for s in &pf.symbols {
            id_map.insert((pf.path.clone(), s.qualified_name.clone()), next);
            next += 1;
        }
    }
    id_map
}

/// Build a real `SymbolIndex` + `Engine` over `files` and resolve the bare
/// (chain-less) `Calls` ref named `target` in `caller`, with the file's `ns`
/// `:require`/`:refer` clauses turned into FileContext imports by the clojure
/// hook.
fn resolve_bare_call(files: &[ParsedFile], caller: usize, target: &str) -> Option<Resolution> {
    let id_map = id_map_for(files);
    let index = SymbolIndex::build(files, &id_map);
    let mut eng_ids = SymbolIdMap::default();
    for pf in files {
        for (i, s) in pf.symbols.iter().enumerate() {
            if let Some(&id) = id_map.get(&(pf.path.clone(), s.qualified_name.clone())) {
                eng_ids.insert((pf.path.clone(), i), id);
            }
        }
    }
    let engine = Engine::build_from_registry(files, &eng_ids, &index, index.type_arena_arc());
    let pf = &files[caller];
    let r = pf
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.chain.is_none() && r.target_name == target)
        .expect("the bare call ref");
    let source = &pf.symbols[r.source_symbol_index];
    let file_ctx = ClojureHooks.build_file_context(pf, None).unwrap();
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: source,
        scope_chain: build_scope_chain(source.scope_path.as_deref()),
        file_package_id: None,
    };
    engine.resolve(&ref_ctx, &file_ctx, &index)
}

// An external `clojure.test` whose path carries the `clojure/test` segment run
// that `file_path_matches_module` aligns against the `clojure.test` namespace.
const EXT_CLOJURE_TEST_PATH: &str = "ext:idx:cache/clojure/test.clj";
const EXT_CLOJURE_TEST_SRC: &str = concat!(
    "(ns clojure.test)\n",
    "(defn is [form] form)\n",
    "(defn deftest [name & body] body)\n"
);

// A user namespace that `:refer`s `is` from `clojure.test` and calls it.
const REFER_CALLER_SRC: &str = concat!(
    "(ns my.app-test\n",
    "  (:require [clojure.test :refer [is deftest]]))\n",
    "(defn run [] (is true))\n"
);

// An UNRELATED namespace that defines its OWN top-level `is` and does not
// `:refer` clojure.test — the bind in REFER_CALLER must not land here.
const OTHER_NS_LOCAL_IS_SRC: &str = concat!(
    "(ns other.helpers)\n",
    "(defn is [x] x)\n"
);

/// Precision guard (a): a bare `is` in a namespace WITH the `:refer` binds to
/// `clojure.test/is`, not to a same-named `is` defined in an unrelated
/// namespace.
#[test]
fn refer_binds_bare_call_to_referred_namespace() {
    let files = vec![
        clj_parsed_file(EXT_CLOJURE_TEST_PATH, EXT_CLOJURE_TEST_SRC),
        clj_parsed_file("other/helpers.clj", OTHER_NS_LOCAL_IS_SRC),
        clj_parsed_file("my/app_test.clj", REFER_CALLER_SRC),
    ];
    let res = resolve_bare_call(&files, 2, "is")
        .expect("bare `is` after `:refer [is]` must bind to clojure.test/is");
    let id_map = id_map_for(&files);
    let referred = id_map[&(EXT_CLOJURE_TEST_PATH.to_string(), "is".to_string())];
    let unrelated = id_map[&("other/helpers.clj".to_string(), "is".to_string())];
    assert_eq!(
        res.target_symbol_id, referred,
        "expected the referred clojure.test/is, not the unrelated namespace's is"
    );
    assert_ne!(res.target_symbol_id, unrelated);
}

/// Precision guard (b): a namespace WITHOUT the `:refer` that defines its own
/// local `is` still binds the bare call locally — the injected-name binding is
/// scope-keyed to the file that declared the `:refer`.
#[test]
fn no_refer_binds_local_is() {
    const LOCAL_CALLER_SRC: &str = concat!(
        "(ns my.local)\n",
        "(defn is [x] x)\n",
        "(defn run [] (is 1))\n"
    );
    let files = vec![
        clj_parsed_file(EXT_CLOJURE_TEST_PATH, EXT_CLOJURE_TEST_SRC),
        clj_parsed_file("my/local.clj", LOCAL_CALLER_SRC),
    ];
    let res = resolve_bare_call(&files, 1, "is")
        .expect("a local `is` with no `:refer` must still bind to the local def");
    let id_map = id_map_for(&files);
    let local = id_map[&("my/local.clj".to_string(), "is".to_string())];
    assert_eq!(
        res.target_symbol_id, local,
        "without `:refer`, bare `is` binds the file's own definition"
    );
}

/// Precision guard (c): `:refer :all` emits no per-name binding, so it produces
/// no non-wildcard import entry — only the whole-namespace wildcard import. The
/// behavior is unchanged from a plain `:require` and is out of scope for the
/// per-name injection.
#[test]
fn refer_all_emits_no_per_name_binding() {
    const REFER_ALL_SRC: &str = concat!(
        "(ns my.all-test\n",
        "  (:require [clojure.test :refer :all]))\n",
        "(defn run [] (is true))\n"
    );
    let pf = clj_parsed_file("my/all_test.clj", REFER_ALL_SRC);
    let file_ctx = ClojureHooks.build_file_context(&pf, None).unwrap();
    // No import entry binds the bare name `is`: `:refer :all` only yields the
    // wildcard namespace import keyed on `clojure.test`.
    assert!(
        !file_ctx
            .imports
            .iter()
            .any(|i| !i.is_wildcard && i.imported_name == "is"),
        "`:refer :all` must not emit a per-name `is` import binding"
    );
    assert!(
        file_ctx
            .imports
            .iter()
            .any(|i| i.is_wildcard && i.module_path.as_deref() == Some("clojure.test")),
        "`:refer :all` keeps the whole-namespace wildcard import"
    );
}

/// Precision guard (d): an `:as` alias import is unchanged — it produces a
/// wildcard namespace import keyed on the namespace (qualified `str/join` calls
/// resolve off the call ref's own module, not through this entry). The `:as`
/// alias must never become a non-wildcard per-name binding.
#[test]
fn as_alias_does_not_regress_to_per_name_binding() {
    const AS_ALIAS_SRC: &str = concat!(
        "(ns my.fmt\n",
        "  (:require [clojure.string :as str]))\n",
        "(defn run [] (str/join [\"a\" \"b\"]))\n"
    );
    let pf = clj_parsed_file("my/fmt.clj", AS_ALIAS_SRC);
    let file_ctx = ClojureHooks.build_file_context(&pf, None).unwrap();
    assert!(
        file_ctx
            .imports
            .iter()
            .any(|i| i.is_wildcard && i.module_path.as_deref() == Some("clojure.string")),
        "`:as` import keeps the whole-namespace wildcard entry"
    );
    assert!(
        !file_ctx.imports.iter().any(|i| !i.is_wildcard),
        "`:as` import must not produce a non-wildcard per-name binding"
    );
}

/// `build_file_context` models a `:refer` entry as an import binding: the
/// injected name is `imported_name`, the namespace is `module_path`, no alias,
/// not a wildcard. This is the shape the bare-name import rung consumes.
#[test]
fn build_file_context_models_refer_as_import_binding() {
    let pf = clj_parsed_file("my/app_test.clj", REFER_CALLER_SRC);
    let file_ctx = ClojureHooks.build_file_context(&pf, None).unwrap();
    let is_entry = file_ctx
        .imports
        .iter()
        .find(|i| i.imported_name == "is")
        .expect("`:refer [is]` must produce an `is` import entry");
    assert_eq!(is_entry.module_path.as_deref(), Some("clojure.test"));
    assert_eq!(is_entry.alias, None);
    assert!(
        !is_entry.is_wildcard,
        "a per-name `:refer` binding is not a wildcard import"
    );
    // Both referred names land as bindings.
    assert!(file_ctx
        .imports
        .iter()
        .any(|i| i.imported_name == "deftest" && !i.is_wildcard));
}
