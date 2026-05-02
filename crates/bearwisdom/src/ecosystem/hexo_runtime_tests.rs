use super::*;
use std::fs;

#[test]
fn looks_like_hexo_via_node_modules() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("node_modules/hexo")).unwrap();
    assert!(looks_like_hexo_project(dir.path()));
}

#[test]
fn looks_like_hexo_via_config_yml() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("_config.yml"), "# Hexo Configuration\ntitle: blog\n").unwrap();
    assert!(looks_like_hexo_project(dir.path()));
}

#[test]
fn arbitrary_directory_is_not_hexo() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("README.md"), "# random").unwrap();
    assert!(!looks_like_hexo_project(dir.path()));
}

#[test]
fn extract_helper_names_picks_up_single_quoted() {
    let mut out = Vec::new();
    extract_helper_names(
        "helper.register('is_home', is.home);\nhelper.register('partial', fn);\n",
        "helper.register",
        &mut out,
    );
    assert!(out.contains(&"is_home".to_string()));
    assert!(out.contains(&"partial".to_string()));
}

#[test]
fn extract_helper_names_picks_up_double_quoted() {
    let mut out = Vec::new();
    extract_helper_names(
        r#"helper.register("url_for", impl);"#,
        "helper.register",
        &mut out,
    );
    assert_eq!(out, vec!["url_for".to_string()]);
}

#[test]
fn extract_helper_names_skips_identifier_suffix() {
    // `xhelper.register` must not match `helper.register`.
    let mut out = Vec::new();
    extract_helper_names(
        "xhelper.register('not_a_helper', fn);",
        "helper.register",
        &mut out,
    );
    assert!(out.is_empty(), "got: {out:?}");
}

#[test]
fn extract_helper_names_dedupes() {
    let mut out = Vec::new();
    extract_helper_names(
        "helper.register('partial', a);\nhelper.register('partial', b);\n",
        "helper.register",
        &mut out,
    );
    assert_eq!(out, vec!["partial".to_string()]);
}

#[test]
fn synthesise_emits_symbols_for_core_helpers() {
    let dir = tempfile::tempdir().unwrap();
    let helper_dir = dir.path().join("node_modules/hexo/dist/plugins/helper");
    fs::create_dir_all(&helper_dir).unwrap();
    fs::write(
        helper_dir.join("index.js"),
        "module.exports = (ctx) => {\n  helper.register('is_home', x);\n  helper.register('partial', y);\n};\n",
    )
    .unwrap();

    let parsed = synthesise_hexo_helpers(dir.path());
    assert_eq!(parsed.len(), 1, "expected one synthetic ParsedFile");
    let names: Vec<&str> = parsed[0].symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"is_home"));
    assert!(names.contains(&"partial"));
    assert!(parsed[0].symbols.iter().all(|s| s.kind == crate::types::SymbolKind::Function));
}

#[test]
fn synthesise_picks_up_theme_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let scripts = dir.path().join("themes/coo/scripts");
    fs::create_dir_all(&scripts).unwrap();
    fs::write(
        scripts.join("global.js"),
        "hexo.extend.helper.register('clean_url', function(url) { return url; });\n",
    )
    .unwrap();
    fs::write(
        scripts.join("github.js"),
        "hexo.extend.helper.register('edit_page', () => {});\nhexo.extend.helper.register('contributing', fn);\n",
    )
    .unwrap();
    // Mark project as Hexo via _config.yml.
    fs::write(dir.path().join("_config.yml"), "# Hexo Configuration\n").unwrap();

    let parsed = synthesise_hexo_helpers(dir.path());
    assert_eq!(parsed.len(), 1);
    let names: Vec<&str> = parsed[0].symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"clean_url"));
    assert!(names.contains(&"edit_page"));
    assert!(names.contains(&"contributing"));
}

#[test]
fn synthesise_returns_empty_when_no_helpers_found() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("_config.yml"), "title: x\n").unwrap();
    let parsed = synthesise_hexo_helpers(dir.path());
    assert!(parsed.is_empty());
}

#[test]
fn helpers_emit_under_npm_globals_namespace() {
    // Hexo helpers must live under `__npm_globals__.<name>` so the
    // TypeScript resolver's `ts_npm_globals` fallback matches calls in
    // both .js sibling scripts and embedded JS inside .ejs templates.
    let dir = tempfile::tempdir().unwrap();
    let helper_dir = dir.path().join("node_modules/hexo/dist/plugins/helper");
    fs::create_dir_all(&helper_dir).unwrap();
    fs::write(helper_dir.join("index.js"), "helper.register('partial', x);\n").unwrap();
    let scripts = dir.path().join("themes/coo/scripts");
    fs::create_dir_all(&scripts).unwrap();
    fs::write(
        scripts.join("g.js"),
        "hexo.extend.helper.register('icon', x);\n",
    )
    .unwrap();

    let parsed = synthesise_hexo_helpers(dir.path());
    for sym in &parsed[0].symbols {
        assert!(
            sym.qualified_name.starts_with("__npm_globals__."),
            "expected __npm_globals__ qname, got: {}",
            sym.qualified_name
        );
    }
    let names: Vec<&str> = parsed[0].symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"partial"));
    assert!(names.contains(&"icon"));

    // Originating scope is preserved on scope_path for downstream tools.
    assert!(parsed[0]
        .symbols
        .iter()
        .any(|s| s.name == "partial" && s.scope_path.as_deref() == Some("hexo.core")));
    assert!(parsed[0]
        .symbols
        .iter()
        .any(|s| s.name == "icon" && s.scope_path.as_deref() == Some("hexo.theme")));
}
