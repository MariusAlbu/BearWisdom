use std::collections::HashMap;

use super::{
    parse_gradle_coords, parse_gradle_dependencies, parse_gradle_direct_coords,
    parse_version_catalog, GradleCatalog,
};

#[test]
fn parse_gradle_dependencies_returns_groupids_for_legacy_callers() {
    let content = r#"
        dependencies {
            implementation 'org.springframework:spring-core:5.3.0'
            api("com.google.guava:guava:31.1-jre")
            testImplementation 'org.junit.jupiter:junit-jupiter:5.10.0'
        }
    "#;
    let groups = parse_gradle_dependencies(content);
    assert!(groups.contains(&"org.springframework".to_string()));
    assert!(groups.contains(&"com.google.guava".to_string()));
    assert!(groups.contains(&"org.junit.jupiter".to_string()));
}

#[test]
fn parse_gradle_direct_coords_extracts_full_gav() {
    let content = r#"
        implementation 'org.assertj:assertj-core:3.27.7'
        api("org.jetbrains.kotlin:kotlin-stdlib:2.2.21")
        kapt 'com.google.dagger:dagger-compiler:2.51'
    "#;
    let coords = parse_gradle_direct_coords(content);
    assert_eq!(coords.len(), 3);
    let assertj = coords.iter().find(|c| c.artifact_id == "assertj-core").unwrap();
    assert_eq!(assertj.group_id, "org.assertj");
    assert_eq!(assertj.version.as_deref(), Some("3.27.7"));
}

#[test]
fn parse_gradle_direct_coords_skips_catalog_refs() {
    let content = r#"
        implementation(libs.assertj.core)
        implementation(libs.kotlin.compiler)
    "#;
    // No string literal — direct parser yields nothing.
    assert!(parse_gradle_direct_coords(content).is_empty());
}

#[test]
fn parse_version_catalog_resolves_version_ref() {
    let content = r#"
[versions]
kotlin = "2.3.20"

[libraries]
kotlin-compiler = { module = "org.jetbrains.kotlin:kotlin-compiler", version.ref = "kotlin" }
assertj-core = { module = "org.assertj:assertj-core", version = "3.27.7" }
"#;
    let cat = parse_version_catalog(content);
    let coord = cat.libraries.get("kotlin.compiler").expect("kotlin.compiler");
    assert_eq!(coord.group_id, "org.jetbrains.kotlin");
    assert_eq!(coord.artifact_id, "kotlin-compiler");
    assert_eq!(coord.version.as_deref(), Some("2.3.20"));

    let assertj = cat.libraries.get("assertj.core").expect("assertj.core");
    assert_eq!(assertj.version.as_deref(), Some("3.27.7"));
}

#[test]
fn parse_version_catalog_handles_group_name_form() {
    let content = r#"
[libraries]
junit = { group = "junit", name = "junit", version = "4.13.2" }
"#;
    let cat = parse_version_catalog(content);
    let coord = cat.libraries.get("junit").expect("junit");
    assert_eq!(coord.group_id, "junit");
    assert_eq!(coord.artifact_id, "junit");
    assert_eq!(coord.version.as_deref(), Some("4.13.2"));
}

#[test]
fn parse_version_catalog_kebab_to_dot_accessor() {
    let content = r#"
[libraries]
kotlinx-coroutinesCore = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version = "1.10.2" }
"#;
    let cat = parse_version_catalog(content);
    // kebab `kotlinx-coroutinesCore` becomes accessor `kotlinx.coroutinesCore`
    // (only `-` flips to `.`, camelCase preserved).
    assert!(cat.libraries.contains_key("kotlinx.coroutinesCore"));
}

#[test]
fn parse_version_catalog_ignores_plugins_and_bundles() {
    let content = r#"
[plugins]
kotlin-jvm = { id = "org.jetbrains.kotlin.jvm", version.ref = "kotlin" }

[bundles]
testing = ["junit", "assertj-core"]

[libraries]
junit = { module = "junit:junit", version = "4.13.2" }
"#;
    let cat = parse_version_catalog(content);
    assert_eq!(cat.libraries.len(), 1);
    assert!(cat.libraries.contains_key("junit"));
}

#[test]
fn parse_gradle_coords_resolves_catalog_references() {
    let mut catalogs = HashMap::new();
    let mut libs = GradleCatalog::default();
    libs.libraries.insert(
        "assertj.core".to_string(),
        crate::ecosystem::manifest::maven::MavenCoord {
            group_id: "org.assertj".to_string(),
            artifact_id: "assertj-core".to_string(),
            version: Some("3.27.7".to_string()),
        },
    );
    catalogs.insert("libs".to_string(), libs);

    let content = r#"
        dependencies {
            testImplementation(libs.assertj.core)
            implementation 'org.jetbrains.kotlin:kotlin-stdlib:2.2.21'
        }
    "#;
    let coords = parse_gradle_coords(content, &catalogs);
    assert_eq!(coords.len(), 2);
    assert!(coords.iter().any(|c| c.artifact_id == "assertj-core"));
    assert!(coords.iter().any(|c| c.artifact_id == "kotlin-stdlib"));
}

#[test]
fn parse_version_catalog_strips_inline_comments() {
    let content = r#"
[versions]
kotlin = "2.3.20" # latest stable

[libraries]
junit = { module = "junit:junit", version = "4.13.2" } # used by tests only
"#;
    let cat = parse_version_catalog(content);
    assert_eq!(cat.versions.get("kotlin").map(|s| s.as_str()), Some("2.3.20"));
    assert!(cat.libraries.contains_key("junit"));
}

#[test]
fn parse_gradle_direct_coords_skips_invalid_lines() {
    let content = r#"
        // implementation 'commented:out:1.0'
        implementation project(":other-module")
        implementation 'org.example:lib:1.0'
    "#;
    let coords = parse_gradle_direct_coords(content);
    assert_eq!(coords.len(), 1);
    assert_eq!(coords[0].artifact_id, "lib");
}
