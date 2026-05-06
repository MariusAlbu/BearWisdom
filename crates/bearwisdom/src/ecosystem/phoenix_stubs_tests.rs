use super::*;

fn all_symbols() -> Vec<ExtractedSymbol> {
    synthesize_all()
        .into_iter()
        .flat_map(|pf| pf.symbols)
        .collect()
}

#[test]
fn put_flash_is_synthesized_under_phoenix_controller() {
    let syms = all_symbols();
    let s = syms.iter().find(|s| s.qualified_name == "Phoenix.Controller.put_flash");
    assert!(s.is_some(), "Phoenix.Controller.put_flash must be present");
    let s = s.unwrap();
    assert_eq!(s.name, "put_flash");
    assert_eq!(s.kind, SymbolKind::Function);
}

#[test]
fn field_is_synthesized_under_ecto_schema() {
    let syms = all_symbols();
    assert!(
        syms.iter().any(|s| s.qualified_name == "Ecto.Schema.field"),
        "Ecto.Schema.field must be present"
    );
}

#[test]
fn live_view_test_assertion_helpers_present() {
    let syms = all_symbols();
    for name in ["render_click", "render_submit", "live", "assert_patch"] {
        let qname = format!("Phoenix.LiveViewTest.{name}");
        assert!(
            syms.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized"
        );
    }
}

#[test]
fn ecto_query_and_repo_terminals_present() {
    let syms = all_symbols();
    for qname in [
        "Ecto.Query.from",
        "Ecto.Query.where",
        "Ecto.Query.preload",
        "Ecto.Repo.all",
        "Ecto.Repo.get",
        "Ecto.Repo.insert",
        "Ecto.Repo.transaction",
    ] {
        assert!(
            syms.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized"
        );
    }
}

#[test]
fn plug_conn_and_phoenix_conn_test_dont_overlap_namespace() {
    // Both modules define `put_session` — they must be separately
    // namespaced so by_qualified_name distinguishes them. The by_name
    // resolver step will still find either when a call is bare.
    let syms = all_symbols();
    assert!(syms.iter().any(|s| s.qualified_name == "Plug.Conn.put_session"));
    assert!(syms.iter().any(|s| s.qualified_name == "Phoenix.ConnTest.put_session"));
}

#[test]
fn every_module_has_namespace_symbol() {
    let pfs = synthesize_all();
    for pf in &pfs {
        let has_ns = pf.symbols.iter().any(|s| s.kind == SymbolKind::Namespace);
        assert!(
            has_ns,
            "synthetic file {} must contain a namespace symbol",
            pf.path
        );
    }
}

#[test]
fn parallel_vecs_are_consistent() {
    for pf in synthesize_all() {
        assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len(),
            "symbol_origin_languages length mismatch for {}", pf.path);
        assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len(),
            "symbol_from_snippet length mismatch for {}", pf.path);
    }
}

#[test]
fn activation_gates_on_mix_exs_phoenix_dep() {
    let e = PhoenixStubsEcosystem;
    assert_eq!(e.languages(), &["elixir"]);
    assert_eq!(e.kind(), EcosystemKind::Stdlib);
    match e.activation() {
        EcosystemActivation::ManifestFieldContains { manifest_glob, field_path, value } => {
            assert_eq!(manifest_glob, "**/mix.exs");
            assert_eq!(field_path, "");
            assert_eq!(value, ":phoenix");
        }
        other => panic!("expected ManifestFieldContains, got {:?}", other),
    }
}
