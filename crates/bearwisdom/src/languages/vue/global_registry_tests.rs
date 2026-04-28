// =============================================================================
// vue/global_registry_tests.rs — unit tests for global_registry.rs
// =============================================================================

use super::*;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// parse_imports
// ---------------------------------------------------------------------------

#[test]
fn parse_default_import_from_element_plus() {
    let src = "import ElementPlus from 'element-plus'";
    let map = _test_parse_imports(src);
    assert_eq!(map.get("ElementPlus").map(String::as_str), Some("element-plus"));
}

#[test]
fn parse_default_import_from_element_ui() {
    let src = "import Element from 'element-ui'";
    let map = _test_parse_imports(src);
    assert_eq!(map.get("Element").map(String::as_str), Some("element-ui"));
}

#[test]
fn parse_named_import() {
    let src = "import { createVuestic } from 'vuestic-ui'";
    let map = _test_parse_imports(src);
    assert_eq!(map.get("createVuestic").map(String::as_str), Some("vuestic-ui"));
}

#[test]
fn parse_multiple_named_imports() {
    let src = "import { NButton, NInput, NSelect } from 'naive-ui'";
    let map = _test_parse_imports(src);
    assert_eq!(map.get("NButton").map(String::as_str), Some("naive-ui"));
    assert_eq!(map.get("NInput").map(String::as_str), Some("naive-ui"));
    assert_eq!(map.get("NSelect").map(String::as_str), Some("naive-ui"));
}

#[test]
fn parse_aliased_import_uses_local_name() {
    let src = "import { Button as ElButton } from 'element-plus'";
    let map = _test_parse_imports(src);
    assert_eq!(map.get("ElButton").map(String::as_str), Some("element-plus"));
    assert!(map.get("Button").is_none());
}

#[test]
fn parse_double_quoted_import() {
    let src = r#"import Vuestic from "vuestic-ui""#;
    let map = _test_parse_imports(src);
    assert_eq!(map.get("Vuestic").map(String::as_str), Some("vuestic-ui"));
}

#[test]
fn parse_default_and_named() {
    let src = "import ElementPlus, { ElLoading } from 'element-plus'";
    let map = _test_parse_imports(src);
    assert_eq!(map.get("ElementPlus").map(String::as_str), Some("element-plus"));
    assert_eq!(map.get("ElLoading").map(String::as_str), Some("element-plus"));
}

#[test]
fn ignore_non_import_lines() {
    let src = "const x = require('element-plus')";
    let map = _test_parse_imports(src);
    assert!(map.is_empty());
}

// ---------------------------------------------------------------------------
// detect_app_use_calls
// ---------------------------------------------------------------------------

#[test]
fn detect_vue3_app_use_element_plus() {
    let src = "import ElementPlus from 'element-plus'\napp.use(ElementPlus)";
    let imports = _test_parse_imports(src);
    let uses = _test_detect_app_use(src, &imports);
    assert_eq!(uses.len(), 1);
    assert_eq!(uses[0].0, "ElementPlus");
    assert_eq!(uses[0].1, "element-plus");
}

#[test]
fn detect_vue2_vue_use_element_ui() {
    let src = "import Element from 'element-ui'\nVue.use(Element)";
    let imports = _test_parse_imports(src);
    let uses = _test_detect_app_use(src, &imports);
    assert_eq!(uses.len(), 1);
    assert_eq!(uses[0].0, "Element");
    assert_eq!(uses[0].1, "element-ui");
}

#[test]
fn detect_vuestic_ui_use() {
    let src = "import { createVuestic } from 'vuestic-ui'\napp.use(createVuestic({ config: {} }))";
    // Note: createVuestic( is nested — the line-based scanner sees `createVuestic({...})`
    // which won't be in the import map since it's a call expression not a bound identifier.
    // The app.use arg extraction gets `createVuestic` which IS in the map.
    let imports = _test_parse_imports(src);
    // createVuestic IS in the import map from 'vuestic-ui'
    assert_eq!(imports.get("createVuestic").map(String::as_str), Some("vuestic-ui"));
    // app.use(createVuestic(...)) — arg extraction gets `createVuestic` before `(`
    let uses = _test_detect_app_use(src, &imports);
    assert_eq!(uses.len(), 1);
    assert_eq!(uses[0].1, "vuestic-ui");
}

#[test]
fn ignore_router_and_store_use() {
    // router and store aren't component libraries
    let src = "import router from './router'\napp.use(router)";
    let imports = _test_parse_imports(src);
    let uses = _test_detect_app_use(src, &imports);
    assert!(uses.is_empty(), "router.use should not be detected as component library");
}

#[test]
fn detect_multiple_use_calls() {
    let src = "import ElementPlus from 'element-plus'\nimport NaiveUI from 'naive-ui'\napp.use(ElementPlus)\napp.use(NaiveUI)";
    let imports = _test_parse_imports(src);
    let uses = _test_detect_app_use(src, &imports);
    assert_eq!(uses.len(), 2);
    let pkgs: Vec<_> = uses.iter().map(|(_, p)| p.as_str()).collect();
    assert!(pkgs.contains(&"element-plus"));
    assert!(pkgs.contains(&"naive-ui"));
}

// ---------------------------------------------------------------------------
// detect_component_registrations
// ---------------------------------------------------------------------------

#[test]
fn detect_app_component_pascal_case() {
    let src = "app.component('SvgIcon', SvgIcon)";
    let names = _test_detect_component_registrations(src);
    assert_eq!(names, vec!["SvgIcon"]);
}

#[test]
fn detect_vue_component_kebab_normalizes_to_pascal() {
    let src = "Vue.component('svg-icon', SvgIcon)";
    let names = _test_detect_component_registrations(src);
    assert_eq!(names, vec!["SvgIcon"]);
}

#[test]
fn detect_app_component_already_pascal() {
    let src = "app.component('MyButton', MyButton)";
    let names = _test_detect_component_registrations(src);
    assert_eq!(names, vec!["MyButton"]);
}

#[test]
fn single_segment_lowercase_normalizes_to_pascal() {
    // 'mywidget' is single-segment kebab-like → capitalizes first char → 'Mywidget'
    // starts uppercase after capitalization, so it is collected
    let src = "app.component('mywidget', Widget)";
    let names = _test_detect_component_registrations(src);
    assert_eq!(names, vec!["Mywidget"]);
}

#[test]
fn detect_multiple_component_registrations() {
    let src = "app.component('FooBar', FooBar)\napp.component('BazQux', BazQux)";
    let names = _test_detect_component_registrations(src);
    assert!(names.contains(&"FooBar".to_string()));
    assert!(names.contains(&"BazQux".to_string()));
}

// ---------------------------------------------------------------------------
// library_for_name
// ---------------------------------------------------------------------------

#[test]
fn library_for_el_prefix_resolves_to_element_plus() {
    let mut registry = VueGlobalRegistry::default();
    registry.components.insert(
        "__prefix__El".to_string(),
        VueComponentSource::Library {
            package: "element-plus".to_string(),
        },
    );
    assert_eq!(library_for_name(&registry, "ElButton"), Some("element-plus"));
    assert_eq!(library_for_name(&registry, "ElTableColumn"), Some("element-plus"));
    assert_eq!(library_for_name(&registry, "ElInput"), Some("element-plus"));
}

#[test]
fn library_for_va_prefix_resolves_to_vuestic() {
    let mut registry = VueGlobalRegistry::default();
    registry.components.insert(
        "__prefix__Va".to_string(),
        VueComponentSource::Library {
            package: "vuestic-ui".to_string(),
        },
    );
    assert_eq!(library_for_name(&registry, "VaButton"), Some("vuestic-ui"));
    assert_eq!(library_for_name(&registry, "VaIcon"), Some("vuestic-ui"));
}

#[test]
fn library_for_no_match_returns_none() {
    let registry = VueGlobalRegistry::default();
    assert!(library_for_name(&registry, "ElButton").is_none());
}

#[test]
fn library_for_exact_component_match() {
    let mut registry = VueGlobalRegistry::default();
    registry.components.insert(
        "SvgIcon".to_string(),
        VueComponentSource::ExplicitRegistration {
            file: "src/icons/index.js".to_string(),
        },
    );
    // ExplicitRegistration returns None from library_for_name
    assert!(library_for_name(&registry, "SvgIcon").is_none());
}

// ---------------------------------------------------------------------------
// Full scan integration test (in-memory project tree)
// ---------------------------------------------------------------------------

#[test]
fn scan_detects_element_ui_from_main_js() {
    let dir = tempdir_or_skip();
    // Write a fake main.js
    let main_js = dir.path().join("main.js");
    std::fs::write(
        &main_js,
        "import Element from 'element-ui'\nVue.use(Element)\n",
    )
    .unwrap();

    let parsed_paths = vec!["main.js".to_string()];
    let registry = scan_global_registrations(dir.path(), &parsed_paths);

    // Should have a prefix sentinel for 'El'
    assert!(
        library_for_name(&registry, "ElButton").is_some(),
        "ElButton should resolve via element-ui prefix"
    );
    assert_eq!(library_for_name(&registry, "ElButton"), Some("element-ui"));
}

#[test]
fn scan_detects_explicit_component_registration() {
    let dir = tempdir_or_skip();
    let main_ts = dir.path().join("main.ts");
    std::fs::write(
        &main_ts,
        "import SvgIcon from './SvgIcon.vue'\napp.component('svg-icon', SvgIcon)\n",
    )
    .unwrap();

    let parsed_paths = vec!["main.ts".to_string()];
    let registry = scan_global_registrations(dir.path(), &parsed_paths);

    assert!(
        registry.components.contains_key("SvgIcon"),
        "SvgIcon should be in registry after explicit registration"
    );
}

#[test]
fn scan_detects_unplugin_in_vite_config() {
    let dir = tempdir_or_skip();
    let vite_config = dir.path().join("vite.config.ts");
    std::fs::write(
        &vite_config,
        "import Components from 'unplugin-vue-components/vite'\nexport default { plugins: [Components({})] }",
    )
    .unwrap();

    let parsed_paths = vec![];
    let registry = scan_global_registrations(dir.path(), &parsed_paths);
    assert!(registry.has_unplugin_auto_import);
}

#[test]
fn scan_returns_empty_for_non_vue_project() {
    let dir = tempdir_or_skip();
    // No entry files, no vite config
    let registry = scan_global_registrations(dir.path(), &[]);
    assert!(registry.is_empty());
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tempdir_or_skip() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("failed to create temp dir")
}
