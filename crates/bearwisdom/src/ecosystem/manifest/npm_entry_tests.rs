// =============================================================================
// npm_entry_tests — package entry candidates in declared priority
// =============================================================================

use super::package_entries;

#[test]
fn exports_dot_leaves_come_first_declarations_ahead_then_legacy_fields() {
    let entries = package_entries(
        r#"{
  "name": "@acme/core",
  "main": "build/legacy/index.cjs",
  "types": "build/legacy/index.d.ts",
  "module": "build/legacy/index.js",
  "exports": {
    ".": {
      "@acme/custom-condition": "./src/index.ts",
      "import": { "types": "./build/modern/index.d.ts", "default": "./build/modern/index.js" },
      "require": { "types": "./build/modern/index.d.cts", "default": "./build/modern/index.cjs" }
    },
    "./package.json": "./package.json"
  }
}"#,
    );
    // Conditions at one level keep the map's key order with `types` leaves
    // ahead; the legacy fields follow in field priority.
    assert_eq!(
        entries,
        vec![
            "src/index.ts",
            "build/modern/index.d.ts",
            "build/modern/index.js",
            "build/modern/index.d.cts",
            "build/modern/index.cjs",
            "build/legacy/index.d.ts",
            "build/legacy/index.cjs",
            "build/legacy/index.js",
        ]
    );
}

#[test]
fn a_string_or_conditions_only_exports_field_is_the_dot_entry() {
    assert_eq!(
        package_entries(r#"{"exports":"./index.js"}"#),
        vec!["index.js"]
    );
    assert_eq!(
        package_entries(r#"{"exports":{"types":"./dist/index.d.ts","default":"./dist/index.js"}}"#),
        vec!["dist/index.d.ts", "dist/index.js"]
    );
    assert_eq!(
        package_entries(r#"{"name":"bare","types":"src/index.ts","main":"src/index.ts"}"#),
        vec!["src/index.ts"]
    );
    assert!(package_entries("{}").is_empty());
    assert!(package_entries("not json").is_empty());
}
