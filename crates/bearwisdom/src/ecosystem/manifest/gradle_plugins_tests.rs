use super::*;

fn catalogs(entries: &[(&str, &str, &str)]) -> PluginCatalogs {
    let mut out = PluginCatalogs::new();
    for (catalog, accessor, id) in entries {
        out.entry((*catalog).to_string())
            .or_default()
            .insert((*accessor).to_string(), (*id).to_string());
    }
    out
}

#[test]
fn kotlin_dsl_id_call_is_an_applied_plugin() {
    let script = "plugins {\n    id(\"com.android.application\")\n}\n";
    assert_eq!(
        applied_plugin_ids(script, &PluginCatalogs::new()),
        vec!["com.android.application".to_string()]
    );
}

#[test]
fn groovy_id_with_a_version_clause_keeps_only_the_id() {
    let script = "plugins {\n    id 'com.android.library' version '8.1.0'\n}\n";
    assert_eq!(
        applied_plugin_ids(script, &PluginCatalogs::new()),
        vec!["com.android.library".to_string()]
    );
}

#[test]
fn backtick_accessor_is_an_applied_plugin() {
    let script = "plugins {\n    `kotlin-dsl`\n}\n";
    assert_eq!(
        applied_plugin_ids(script, &PluginCatalogs::new()),
        vec!["kotlin-dsl".to_string()]
    );
}

#[test]
fn catalog_alias_resolves_through_the_plugins_table() {
    let script = "plugins {\n    alias(libs.plugins.android.application)\n}\n";
    let cats = catalogs(&[("libs", "android.application", "com.android.application")]);
    assert_eq!(
        applied_plugin_ids(script, &cats),
        vec!["com.android.application".to_string()]
    );
}

#[test]
fn unknown_catalog_alias_yields_nothing() {
    let script = "plugins {\n    alias(libs.plugins.spotless)\n}\n";
    assert!(applied_plugin_ids(script, &PluginCatalogs::new()).is_empty());
}

#[test]
fn legacy_apply_statement_is_an_applied_plugin() {
    let groovy = "apply plugin: 'com.android.application'\n";
    assert_eq!(
        applied_plugin_ids(groovy, &PluginCatalogs::new()),
        vec!["com.android.application".to_string()]
    );
    let kotlin = "apply(plugin = \"com.android.library\")\n";
    assert_eq!(
        applied_plugin_ids(kotlin, &PluginCatalogs::new()),
        vec!["com.android.library".to_string()]
    );
}

#[test]
fn a_dependency_coordinate_is_not_an_applied_plugin() {
    // The shape that made a plain JVM build look like an Android one: the
    // namespace appears in an exclusion and on the buildscript classpath,
    // and neither applies anything to this module.
    let script = "dependencies {\n    implementation('org.ow2.asm:asm:9.7') {\n        \
                  exclude group: 'com.android.tools'\n    }\n}\n\
                  buildscript {\n    dependencies {\n        \
                  classpath 'com.android.tools.build:gradle:8.1.0'\n    }\n}\n";
    assert!(applied_plugin_ids(script, &PluginCatalogs::new()).is_empty());
}

#[test]
fn commented_out_plugin_is_not_applied() {
    let script = "plugins {\n    // id(\"com.android.application\")\n    id(\"java\")\n}\n";
    assert_eq!(
        applied_plugin_ids(script, &PluginCatalogs::new()),
        vec!["java".to_string()]
    );
}

#[test]
fn a_qualified_plugins_reference_does_not_open_a_block() {
    let script = "dependencies {\n    implementation(libs.plugins.something)\n}\n";
    assert!(applied_plugin_ids(script, &PluginCatalogs::new()).is_empty());
}

#[test]
fn every_plugins_block_contributes() {
    let script = "plugins {\n    id(\"java\")\n}\n\nsubprojects {\n}\n\n\
                  plugins {\n    id(\"com.android.library\")\n}\n";
    assert_eq!(
        applied_plugin_ids(script, &PluginCatalogs::new()),
        vec!["java".to_string(), "com.android.library".to_string()]
    );
}

#[test]
fn plugin_catalog_reads_both_declaration_forms() {
    let toml = "[versions]\nagp = \"8.1.0\"\n\n\
                [plugins]\n\
                android-application = { id = \"com.android.application\", version.ref = \"agp\" }\n\
                spotless = \"com.diffplug.spotless:6.25.0\"\n\n\
                [libraries]\nguava = { module = \"com.google.guava:guava\", version = \"33.0.0\" }\n";
    let catalog = parse_plugin_catalog(toml);
    assert_eq!(
        catalog.get("android.application").map(String::as_str),
        Some("com.android.application")
    );
    assert_eq!(
        catalog.get("spotless").map(String::as_str),
        Some("com.diffplug.spotless")
    );
    assert!(catalog.get("guava").is_none());
}

#[test]
fn accessor_spelling_joins_both_toml_separators() {
    assert_eq!(
        accessor_spelling("android-application"),
        "android.application"
    );
    assert_eq!(
        accessor_spelling("android_application"),
        "android.application"
    );
    assert_eq!(
        accessor_spelling("androidApplication"),
        "androidApplication"
    );
}

#[test]
fn line_comment_is_stripped_from_the_tail() {
    assert_eq!(strip_line_comment("id(\"java\") // keep"), "id(\"java\") ");
    assert_eq!(strip_line_comment("id(\"java\")"), "id(\"java\")");
}
