// =============================================================================
// tsconfig_paths_tests — wildcard prefixes and exact entries stay apart
// =============================================================================

use super::*;
use std::collections::HashMap;

#[test]
fn wildcard_entries_are_prefixes_and_exact_entries_are_exact() {
    let aliases = parse_tsconfig_aliases(
        r#"{
  // JSONC is fine
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["./src/*"],
      "e2e-utils/*": ["./test/lib/e2e-utils/*"],
      "next-test-utils": ["./test/lib/next-test-utils"],
      "router-act": ["./test/lib/router-act", "./fallback"],
      "odd/*/x": ["./nope/*"],
      "*": ["./anything/*"]
    }
  }
}"#,
    );
    assert_eq!(
        aliases.prefixes,
        vec![
            ("@/".to_string(), "./src/".to_string()),
            ("e2e-utils/".to_string(), "./test/lib/e2e-utils/".to_string()),
        ],
        "an empty alias prefix and a non-trailing wildcard are skipped"
    );
    assert_eq!(
        aliases.exact,
        vec![
            (
                "next-test-utils".to_string(),
                "./test/lib/next-test-utils".to_string()
            ),
            ("router-act".to_string(), "./test/lib/router-act".to_string()),
        ],
        "the first target wins"
    );
    assert_eq!(parse_tsconfig_paths("{}"), Vec::<(String, String)>::new());
}

fn aliases_via(start: &Path, files: Vec<(PathBuf, &str)>) -> TsconfigAliases {
    let map: HashMap<PathBuf, String> = files
        .into_iter()
        .map(|(p, c)| (p, c.to_string()))
        .collect();
    let mut out = TsconfigAliases::default();
    let mut seen = std::collections::HashSet::new();
    collect_tsconfig_paths(start, &|p| map.get(p).cloned(), &mut out, &mut seen, 0);
    out
}

#[test]
fn a_child_config_shadows_its_base_per_key_in_both_axes() {
    let child = PathBuf::from("/p/tsconfig.json");
    let base = PathBuf::from("/p/tsconfig.base.json");
    let out = aliases_via(
        &child,
        vec![
            (
                child.clone(),
                r#"{"extends":"./tsconfig.base.json","compilerOptions":{"paths":{"@/*":["app/*"],"utils":["./app/utils"]}}}"#,
            ),
            (
                base,
                r#"{"compilerOptions":{"paths":{"@/*":["src/*"],"~/*":["lib/*"],"utils":["./src/utils"],"log":["./src/log"]}}}"#,
            ),
        ],
    );
    assert_eq!(
        out.prefixes,
        vec![
            ("@/".to_string(), "app/".to_string()),
            ("~/".to_string(), "lib/".to_string())
        ]
    );
    assert_eq!(
        out.exact,
        vec![
            ("utils".to_string(), "./app/utils".to_string()),
            ("log".to_string(), "./src/log".to_string())
        ]
    );
}
