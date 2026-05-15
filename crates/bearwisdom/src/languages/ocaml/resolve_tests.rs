use super::*;
use crate::types::*;

#[test]
fn test_ocaml_dream_get_emits_consumer() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod};
    let args = vec![CallArg::StringLit("/api/users".to_string())];
    match detect_ocaml_dream_route("Dream", "get", &args).unwrap() {
        FlowEmission::NamedChannel { role, method, .. } => {
            assert_eq!(role, ChannelRole::Consumer);
            assert_eq!(method, Some(HttpMethod::Get));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_ocaml_opium_post_works() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_ocaml_dream_route("App", "post", &args).is_some());
}

#[test]
fn test_ocaml_route_rejects_unknown_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_ocaml_dream_route("Stdlib", "get", &args).is_none());
}

#[test]
fn test_ocaml_cohttp_producer_emits() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission};
    let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
    match detect_ocaml_cohttp_producer("Cohttp_lwt_unix.Client", "get", &args).unwrap() {
        FlowEmission::NamedChannel { role, .. } => assert_eq!(role, ChannelRole::Producer),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_ocaml_cohttp_rejects_non_http_module() {
    let args = vec![CallArg::StringLit("/x".to_string())];
    assert!(detect_ocaml_cohttp_producer("List", "get", &args).is_none());
}

#[test]
fn test_ocaml_caqti_find_emits_db_select() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_ocaml_caqti_emission("Caqti_lwt", "find").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_ocaml_caqti_exec_emits_other() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    match detect_ocaml_caqti_emission("Caqti_lwt_unix", "exec").unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Other),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_ocaml_caqti_via_alias_resolves() {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let aliases = vec![("Q".to_string(), "Caqti_request.Infix".to_string())];
    match detect_ocaml_caqti_with_imports("Q", "find", &aliases).unwrap() {
        FlowEmission::DbQuery { operation, .. } => assert_eq!(operation, DbQueryOp::Select),
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_ocaml_caqti_via_dotted_alias_root() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    // `module Q = Caqti_request.Infix; let _ = Q.find_user ...` — the
    // module string at call site is "Q.find_user", root is "Q".
    let aliases = vec![("Q".to_string(), "Caqti_request.Infix".to_string())];
    match detect_ocaml_caqti_with_imports("Q.find_user", "find", &aliases).unwrap() {
        FlowEmission::DbQuery { .. } => {}
        _ => panic!("expected DbQuery"),
    }
}

#[test]
fn test_ocaml_caqti_short_module_db_aliased() {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    // Db is a canonical short name we accept even without an alias declaration.
    assert!(matches!(
        detect_ocaml_caqti_with_imports("Db", "find", &[]).unwrap(),
        FlowEmission::DbQuery { .. }
    ));
}

#[test]
fn test_ocaml_caqti_no_match_when_alias_resolves_elsewhere() {
    let aliases = vec![("Foo".to_string(), "List.Make".to_string())];
    assert!(detect_ocaml_caqti_with_imports("Foo", "find", &aliases).is_none());
}

#[test]
fn build_file_context_includes_implicit_stdlib_open() {
    use crate::indexer::resolve::engine::LanguageResolver;
    use crate::types::{FlowMeta, ParsedFile};

    let file = ParsedFile {
        path: "src/main.ml".to_string(),
        language: "ocaml".to_string(),
        content_hash: "x".to_string(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };
    let ctx = OcamlResolver.build_file_context(&file, None);
    let stdlib_imports: Vec<_> = ctx
        .imports
        .iter()
        .filter(|i| i.imported_name == "Stdlib")
        .collect();
    assert_eq!(stdlib_imports.len(), 1, "expected exactly one implicit Stdlib open");
    let imp = stdlib_imports[0];
    assert!(imp.is_wildcard, "Stdlib must be wildcard-opened");
    assert_eq!(imp.module_path.as_deref(), Some("stdlib"),
        "module_path lowercased so file_stem_matches against stdlib.ml");
}
