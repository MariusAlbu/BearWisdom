use super::*;

fn parsed_file(path: &str, language: &str) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: language.to_string(),
        content_hash: String::new(),
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
        flow: Default::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }
}

#[test]
fn internal_file_paths_excludes_ext_prefixed() {
    let parsed = vec![
        parsed_file("lib/foo.dart", "dart"),
        parsed_file("ext:dart:pkg/x.dart", "dart"),
    ];
    let paths = internal_file_paths(&parsed);
    assert_eq!(paths, vec!["lib/foo.dart".to_string()]);
}

#[test]
fn resolve_via_module_resolver_bare_relative_dart() {
    let file_paths = vec!["lib/foo.dart".to_string(), "lib/main.dart".to_string()];
    let names: FxHashMap<String, i64> = FxHashMap::default();
    assert_eq!(
        resolve_via_module_resolver(
            "dart",
            "lib/main.dart",
            "foo.dart",
            None,
            &names,
            None,
            &[],
            &file_paths,
        ),
        Some("lib/foo.dart".to_string())
    );
}

#[test]
fn resolve_via_module_resolver_package_self_uses_owning_package_name() {
    let file_paths = vec![
        "lib/src/models/user.dart".to_string(),
        "lib/main.dart".to_string(),
    ];
    let names: FxHashMap<String, i64> = [("app".to_string(), 1)].into_iter().collect();
    assert_eq!(
        resolve_via_module_resolver(
            "dart",
            "lib/main.dart",
            "package:app/src/models/user.dart",
            Some(1),
            &names,
            None,
            &[],
            &file_paths,
        ),
        Some("lib/src/models/user.dart".to_string())
    );
}

#[test]
fn resolve_via_module_resolver_declines_when_package_id_unmatched() {
    let file_paths = vec!["lib/src/models/user.dart".to_string()];
    let names: FxHashMap<String, i64> = [("app".to_string(), 1)].into_iter().collect();
    assert_eq!(
        resolve_via_module_resolver(
            "dart",
            "lib/main.dart",
            "package:app/src/models/user.dart",
            Some(2), // wrong package id — not "app"'s owner
            &names,
            None,
            &[],
            &file_paths,
        ),
        None
    );
}
