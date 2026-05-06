use super::*;

#[test]
fn mock_mvc_request_builders_present() {
    let pf = synthesize_file();
    for name in ["get", "post", "put", "delete", "patch"] {
        let qname = format!(
            "org.springframework.test.web.servlet.request.MockMvcRequestBuilders.{name}"
        );
        let s = pf.symbols.iter().find(|s| s.qualified_name == qname);
        assert!(s.is_some(), "{qname} must be synthesized");
        assert_eq!(s.unwrap().name, name);
        assert_eq!(s.unwrap().kind, SymbolKind::Function);
    }
}

#[test]
fn status_matchers_cover_common_http_codes() {
    let pf = synthesize_file();
    for name in [
        "isOk", "isCreated", "isNoContent",
        "isBadRequest", "isUnauthorized", "isForbidden", "isNotFound",
        "isInternalServerError",
    ] {
        let qname = format!(
            "org.springframework.test.web.servlet.result.StatusResultMatchers.{name}"
        );
        assert!(
            pf.symbols.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized"
        );
    }
}

#[test]
fn jsonpath_matchers_present() {
    let pf = synthesize_file();
    for name in ["value", "exists", "isArray", "isEmpty", "isNotEmpty"] {
        let qname = format!(
            "org.springframework.test.web.servlet.result.JsonPathResultMatchers.{name}"
        );
        assert!(
            pf.symbols.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized"
        );
    }
}

#[test]
fn result_actions_methods_present() {
    let pf = synthesize_file();
    for name in ["andExpect", "andDo", "andReturn"] {
        let qname = format!("org.springframework.test.web.servlet.ResultActions.{name}");
        assert!(
            pf.symbols.iter().any(|s| s.qualified_name == qname),
            "{qname} must be synthesized"
        );
    }
}

#[test]
fn top_level_static_factory_names_match_by_name() {
    let pf = synthesize_file();
    for bare_name in ["get", "post", "status", "jsonPath", "isOk", "isForbidden",
                       "andExpect", "content", "header"] {
        assert!(
            pf.symbols.iter().any(|s| s.name == bare_name),
            "bare name '{bare_name}' must be findable via by_name lookup"
        );
    }
}

#[test]
fn parallel_vecs_are_consistent() {
    let pf = synthesize_file();
    assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len());
    assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len());
}

#[test]
fn activation_gates_on_spring_in_jvm_manifest() {
    let e = SpringStubsEcosystem;
    assert_eq!(e.languages(), &["kotlin", "java"]);
    assert_eq!(e.kind(), EcosystemKind::Stdlib);
    match e.activation() {
        EcosystemActivation::Any(clauses) => {
            let mut globs: Vec<&str> = Vec::new();
            for c in clauses {
                if let EcosystemActivation::ManifestFieldContains { manifest_glob, value, .. } = c {
                    assert_eq!(*value, "org.springframework");
                    globs.push(manifest_glob);
                }
            }
            assert!(globs.contains(&"**/pom.xml"));
            assert!(globs.contains(&"**/build.gradle"));
            assert!(globs.contains(&"**/build.gradle.kts"));
        }
        other => panic!("expected Any(ManifestFieldContains, ...), got {:?}", other),
    }
}
