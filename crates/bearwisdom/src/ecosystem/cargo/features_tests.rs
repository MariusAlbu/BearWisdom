use super::*;

#[test]
fn inline_table_single_line_features() {
    let toml = r#"
[dependencies]
serde = "1"
windows = { version = "0.61", features = ["Win32_Foundation", "Win32_UI"] }
tokio = { version = "1" }
"#;
    let pairs = parse_dependency_features(toml);
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].0, "windows");
    assert_eq!(
        pairs[0].1,
        vec!["Win32_Foundation".to_string(), "Win32_UI".to_string()]
    );
}

#[test]
fn inline_table_multi_line_features() {
    // The shape the spec fixture uses: `name = {` on one line, the `features`
    // array spread across following lines, closing `}` last.
    let toml = r#"
[dependencies]
windows = { version = "0.58.0", features = [
  "Win32_Graphics_Dwm",
  "Win32_Foundation",
  "Win32_UI_Controls",
] }
"#;
    let pairs = parse_dependency_features(toml);
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].0, "windows");
    assert_eq!(
        pairs[0].1,
        vec![
            "Win32_Graphics_Dwm".to_string(),
            "Win32_Foundation".to_string(),
            "Win32_UI_Controls".to_string(),
        ]
    );
}

#[test]
fn sub_table_form_features() {
    let toml = r#"
[dependencies.windows]
version = "0.61"
features = ["Win32_System", "Win32_Foundation"]
"#;
    let pairs = parse_dependency_features(toml);
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].0, "windows");
    assert_eq!(
        pairs[0].1,
        vec!["Win32_System".to_string(), "Win32_Foundation".to_string()]
    );
}

#[test]
fn target_cfg_dependency_features() {
    // Windows-only deps live under `[target.'cfg(windows)'.dependencies]`.
    let toml = r#"
[target.'cfg(windows)'.dependencies]
windows = { version = "0.61", features = ["Win32_Foundation"] }
"#;
    let pairs = parse_dependency_features(toml);
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].0, "windows");
    assert_eq!(pairs[0].1, vec!["Win32_Foundation".to_string()]);
}

#[test]
fn dependency_without_features_yields_nothing() {
    let toml = r#"
[dependencies]
serde = "1"
tokio = { version = "1", default-features = false }
"#;
    assert!(parse_dependency_features(toml).is_empty());
}

#[test]
fn collect_unions_features_across_workspace_members() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let member_a = root.join("crates").join("a");
    let member_b = root.join("crates").join("b");
    std::fs::create_dir_all(&member_a).unwrap();
    std::fs::create_dir_all(&member_b).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
    std::fs::write(
        member_a.join("Cargo.toml"),
        "[dependencies]\nwindows = { version = \"0.61\", features = [\"Win32_Foundation\"] }\n",
    )
    .unwrap();
    std::fs::write(
        member_b.join("Cargo.toml"),
        "[dependencies]\nwindows = { version = \"0.61\", features = [\"Win32_UI_Controls\"] }\n",
    )
    .unwrap();

    let map = _test_collect_crate_features(root);
    let windows = map.get("windows").expect("windows entry");
    assert!(windows.contains(&"Win32_Foundation".to_string()), "{windows:?}");
    assert!(
        windows.contains(&"Win32_UI_Controls".to_string()),
        "{windows:?}"
    );
}

#[test]
fn collect_skips_target_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let target_dep = root.join("target").join("debug").join("build").join("x");
    std::fs::create_dir_all(&target_dep).unwrap();
    // A stray manifest under target/ must not leak its features into the map.
    std::fs::write(
        target_dep.join("Cargo.toml"),
        "[dependencies]\nwindows = { version = \"0.61\", features = [\"ShouldNotAppear\"] }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[dependencies]\nwindows = { version = \"0.61\", features = [\"Win32_Foundation\"] }\n",
    )
    .unwrap();

    let map = _test_collect_crate_features(root);
    let windows = map.get("windows").expect("windows entry");
    assert!(windows.contains(&"Win32_Foundation".to_string()));
    assert!(!windows.contains(&"ShouldNotAppear".to_string()));
}
