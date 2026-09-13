use super::*;

fn versions(entries: &[(&str, &str, &str)]) -> VersionCatalogs {
    let mut out = VersionCatalogs::new();
    for (catalog, accessor, literal) in entries {
        out.entry((*catalog).to_string())
            .or_default()
            .insert((*accessor).to_string(), (*literal).to_string());
    }
    out
}

#[test]
fn kotlin_dsl_assignment_pins_the_level() {
    let script = "android {\n    compileSdk = 35\n}\n";
    assert_eq!(compile_sdk_pin(script, &VersionCatalogs::new()), Some(35));
}

#[test]
fn groovy_bare_accessor_pins_the_level() {
    let script = "android {\n    compileSdk 34\n}\n";
    assert_eq!(compile_sdk_pin(script, &VersionCatalogs::new()), Some(34));
}

#[test]
fn legacy_accessor_pins_the_level_in_both_call_shapes() {
    assert_eq!(
        compile_sdk_pin("compileSdkVersion(29)\n", &VersionCatalogs::new()),
        Some(29)
    );
    assert_eq!(
        compile_sdk_pin("compileSdkVersion 29\n", &VersionCatalogs::new()),
        Some(29)
    );
}

#[test]
fn platform_directory_name_pins_the_level() {
    let script = "compileSdkVersion \"android-30\"\n";
    assert_eq!(compile_sdk_pin(script, &VersionCatalogs::new()), Some(30));
}

#[test]
fn catalog_version_reference_pins_the_level() {
    let script = "android {\n    compileSdk = libs.versions.android.compileSdk.get().toInt()\n}\n";
    let cats = versions(&[("libs", "android.compileSdk", "36")]);
    assert_eq!(compile_sdk_pin(script, &cats), Some(36));
}

#[test]
fn unknown_catalog_reference_pins_nothing() {
    let script = "android {\n    compileSdk = libs.versions.android.compileSdk.get().toInt()\n}\n";
    assert_eq!(compile_sdk_pin(script, &VersionCatalogs::new()), None);
}

#[test]
fn a_longer_accessor_is_not_the_compile_sdk_pin() {
    let script = "android {\n    compileSdkPreview = \"VanillaIceCream\"\n}\n";
    assert_eq!(compile_sdk_pin(script, &VersionCatalogs::new()), None);
}

#[test]
fn commented_out_pin_is_ignored_and_the_live_one_wins() {
    let script = "android {\n    // compileSdk = 21\n    compileSdk = 35\n}\n";
    assert_eq!(compile_sdk_pin(script, &VersionCatalogs::new()), Some(35));
}

#[test]
fn an_unpinned_script_yields_nothing() {
    let script = "plugins {\n    id(\"com.android.library\")\n}\n";
    assert_eq!(compile_sdk_pin(script, &VersionCatalogs::new()), None);
}

#[test]
fn catalog_versions_load_under_their_dsl_spelling() {
    let tmp = tempfile::TempDir::new().unwrap();
    let catalog = tmp.path().join("libs.versions.toml");
    std::fs::write(
        &catalog,
        "[versions]\nandroid-compileSdk = \"36\"\n\n[libraries]\n",
    )
    .unwrap();

    let loaded = load_version_catalogs(&[("libs".to_string(), catalog)]);
    assert_eq!(
        loaded
            .get("libs")
            .and_then(|v| v.get("android.compileSdk"))
            .map(String::as_str),
        Some("36")
    );
}
