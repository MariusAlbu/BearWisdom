use super::hooks::{
    detect_haskell_http_producer, detect_haskell_persistent_emission, detect_haskell_scotty_route,
};
use crate::types::*;

// ---------------------------------------------------------------------------
// End-to-end: constraint-introduced type variables resolve via the generic
// engine's `engine_generic_param` strategy — the same rung that binds C++
// template params and every other declared-generic. No Haskell-specific
// resolver code; the binding is pure extractor data + the shared ladder.
// ---------------------------------------------------------------------------

/// Build a single-file `ParsedFile` from real Haskell extraction output.
fn parsed_haskell(path: &str, src: &str) -> ParsedFile {
    let r = crate::languages::haskell::extract::extract(src);
    ParsedFile {
        path: path.to_string(),
        language: "haskell".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: r.symbols,
        refs: r.refs,
        routes: r.routes,
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: r.alias_targets,
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

/// Build a SymbolIndex over the files, faking the symbol_id_map the indexer
/// normally produces (1-based, per (path, qname)).
fn build_index(files: &[ParsedFile]) -> crate::indexer::resolve::legacy::SymbolIndex {
    let mut id_map: std::collections::HashMap<(String, String), i64> =
        std::collections::HashMap::new();
    let mut next: i64 = 1;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next);
            next += 1;
        }
    }
    crate::indexer::resolve::legacy::SymbolIndex::build(files, &id_map)
}

/// Drive the default resolver over the first ref matching `target`/`kind`
/// inside `pf`, returning the Resolution (or None when the ladder declines).
fn resolve_ref(
    pf: &ParsedFile,
    target: &str,
    kind: EdgeKind,
) -> Option<crate::indexer::resolve::legacy::Resolution> {
    resolve_ref_impl(pf, target, kind, None)
}

/// Same as `resolve_ref` but selects the ref whose source symbol has the given
/// simple name — used to drive a call issued from a specific enclosing scope
/// when several refs share a target name.
fn resolve_ref_from(
    pf: &ParsedFile,
    source_name: &str,
    target: &str,
    kind: EdgeKind,
) -> Option<crate::indexer::resolve::legacy::Resolution> {
    resolve_ref_impl(pf, target, kind, Some(source_name))
}

fn resolve_ref_impl(
    pf: &ParsedFile,
    target: &str,
    kind: EdgeKind,
    source_name: Option<&str>,
) -> Option<crate::indexer::resolve::legacy::Resolution> {
    use crate::indexer::resolve::legacy::{build_scope_chain, RefContext};
    use crate::type_checker::core::DefaultResolver;
    use crate::type_checker::profile::hooks::LanguageEngineHooks;

    let ref_idx = pf
        .refs
        .iter()
        .position(|r| {
            r.target_name == target
                && r.kind == kind
                && source_name
                    .is_none_or(|sn| pf.symbols[r.source_symbol_index].name == sn)
        })
        .expect("ref present");
    let er = &pf.refs[ref_idx];
    let source = &pf.symbols[er.source_symbol_index];

    let files = std::slice::from_ref(pf);
    let index = build_index(files);
    let file_ctx = super::hooks::HaskellHooks
        .build_file_context(pf, None)
        .expect("haskell file context");
    let ref_ctx = RefContext {
        extracted_ref: er,
        source_symbol: source,
        scope_chain: build_scope_chain(source.scope_path.as_deref()),
        file_package_id: None,
    };
    DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &index,
        kind_compatible: |_, _| true,
    }
    .resolve_all_with_profile(&super::profile::HASKELL_PROFILE)
}

#[test]
fn constraint_tyvar_resolves_via_engine_generic_param() {
    // `f :: Ord a => a -> a -> Bool` — the constraint occurrence of tyvar `a`
    // is emitted as a TypeRef from `f`; the extractor stamps `a` onto
    // `f.generic_params`; the merge in the index build seeds the string
    // generic_params map from that Vec; and the shared ladder's
    // `resolve_via_generic_param` rung binds the ref to `f` at confidence 1.0.
    let pf = parsed_haskell("src/M.hs", "f :: Ord a => a -> a -> Bool\nf x y = x == y\n");
    let res = resolve_ref(&pf, "a", EdgeKind::TypeRef).expect("constraint tyvar `a` must resolve");
    assert_eq!(res.strategy, "engine_generic_param");
    assert!(
        res.confidence >= 1.0,
        "expected confidence 1.0; got {}",
        res.confidence
    );

    // The generic-param strategy only ever resolves a tyvar to its own
    // declaring symbol (the ref's source qname), so the bind cannot be a
    // coincidental name match. Confirm the resolved id is one of the `f`
    // symbols, never some unrelated symbol.
    let f_ids: Vec<i64> = pf
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.name == "f")
        .map(|(i, _)| (i + 1) as i64)
        .collect();
    assert!(
        f_ids.contains(&res.target_symbol_id),
        "must bind to an `f` symbol; resolved id {} not in {:?}",
        res.target_symbol_id,
        f_ids
    );
}

#[test]
fn forall_tyvar_resolves_via_engine_generic_param() {
    // The explicit `forall a b.` quantifier supplies the tyvars; the
    // constraint occurrence of `a` still binds through the generic rung.
    let pf = parsed_haskell("src/M.hs", "h :: forall a b. (Eq a) => a -> b -> Bool\n");
    let res =
        resolve_ref(&pf, "a", EdgeKind::TypeRef).expect("forall constraint tyvar `a` must resolve");
    assert_eq!(res.strategy, "engine_generic_param");
}

#[test]
fn test_haskell_scotty_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_haskell_scotty_route("get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_haskell_scotty_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_haskell_scotty_route("post", &args).is_some());
}

#[test]
fn test_haskell_scotty_rejects_unknown() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_haskell_scotty_route("middleware", &args).is_none());
}

#[test]
fn test_haskell_http_producer_emits() {
    use crate::indexer::resolve::flow_emit::ChannelRole;
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    if let crate::indexer::resolve::flow_emit::FlowEmission::NamedChannel { role, .. } =
        detect_haskell_http_producer("Network.Wreq", "get", &args).unwrap()
    {
        assert_eq!(role, ChannelRole::Producer);
    } else {
        panic!("expected NamedChannel");
    }
}

#[test]
fn test_haskell_http_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_haskell_http_producer("Data.List", "get", &args).is_none());
}

#[test]
fn test_haskell_persistent_select_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_haskell_persistent_emission("Database.Persist.Sql", "selectList").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_haskell_persistent_insert_emits_db_insert() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_haskell_persistent_emission("Database.Persist", "insert").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Insert),
        _ => panic!("expected DbQuery"),
    }
}

// ---------------------------------------------------------------------------
// M5 — where-bound locals & operator definitions
// ---------------------------------------------------------------------------

#[test]
fn where_bound_local_call_resolves_to_the_local() {
    // `f x = g x where g y = y + 1` — the body call `g x` must bind to the
    // where-bound helper `g`, the only `g` in the file.
    let pf = parsed_haskell("src/M.hs", "f x = g x\n  where g y = y + 1\n");
    let res =
        resolve_ref(&pf, "g", EdgeKind::Calls).expect("call to where-bound `g` must resolve");

    let g_id = pf
        .symbols
        .iter()
        .position(|s| s.name == "g")
        .map(|i| (i + 1) as i64)
        .expect("where-bound `g` symbol present");
    assert_eq!(
        res.target_symbol_id, g_id,
        "call `g x` must bind to the where-bound `g`; got id {} (strategy {})",
        res.target_symbol_id, res.strategy
    );
}

#[test]
fn operator_definition_call_resolves() {
    // `($$) a b = a` defines the `$$` operator; `h = x $$ y` calls it infix.
    // The call ref carries the bare surface form `$$` and must bind to the
    // operator definition symbol.
    let pf = parsed_haskell(
        "src/M.hs",
        "($$) :: Doc -> Doc -> Doc\n($$) a b = a\nh = x $$ y\n",
    );
    let res = resolve_ref_from(&pf, "h", "$$", EdgeKind::Calls)
        .expect("infix call to `$$` must resolve to the operator definition");

    let op_ids: Vec<i64> = pf
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.name == "$$")
        .map(|(i, _)| (i + 1) as i64)
        .collect();
    assert!(
        op_ids.contains(&res.target_symbol_id),
        "call `x $$ y` must bind to a `$$` symbol; resolved id {} not in {:?}",
        res.target_symbol_id,
        op_ids
    );
}

#[test]
fn where_bound_local_shadows_top_level_for_inner_call() {
    // Scope-proximity pin. A where-bound `g` (under `f`) must win over a
    // top-level `g` for a call issued from a sibling where-bound scope; a call
    // from an unrelated top-level function still binds the top-level `g`.
    let src = "g x = x\n\
               f a = caller a\n\
               \x20 where\n\
               \x20   g y = y\n\
               \x20   caller z = g z\n\
               other z = g z\n";
    let pf = parsed_haskell("src/M.hs", src);

    // Inner call from `caller` (scope_path `f`) — binds the where-bound `f.g`.
    let inner = resolve_ref_from(&pf, "caller", "g", EdgeKind::Calls)
        .expect("inner call to `g` must resolve");
    let where_g_id = pf
        .symbols
        .iter()
        .position(|s| s.name == "g" && s.qualified_name == "f.g")
        .map(|i| (i + 1) as i64)
        .expect("where-bound `f.g` present");
    assert_eq!(
        inner.target_symbol_id, where_g_id,
        "inner call must bind the where-bound `f.g`; got id {} (strategy {})",
        inner.target_symbol_id, inner.strategy
    );

    // Outer call from `other` (top-level, no scope) — binds the top-level `g`.
    let outer = resolve_ref_from(&pf, "other", "g", EdgeKind::Calls)
        .expect("outer call to `g` must resolve");
    let top_g_id = pf
        .symbols
        .iter()
        .position(|s| s.name == "g" && s.qualified_name == "g")
        .map(|i| (i + 1) as i64)
        .expect("top-level `g` present");
    assert_eq!(
        outer.target_symbol_id, top_g_id,
        "outer call must bind the top-level `g`; got id {} (strategy {})",
        outer.target_symbol_id, outer.strategy
    );
}
