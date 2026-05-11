use super::*;

#[test]
fn ecosystem_identity() {
    let n = NimbleEcosystem;
    assert_eq!(n.id(), ID);
    assert_eq!(Ecosystem::kind(&n), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&n), &["nim"]);
}

#[test]
fn legacy_locator_tag_is_nim() {
    assert_eq!(ExternalSourceLocator::ecosystem(&NimbleEcosystem), "nim");
}

#[test]
fn nim_parses_nimble_requires_simple() {
    let tmp = std::env::temp_dir().join("bw-test-nimble-parse-simple");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("test.nimble"), r#"
requires "nim >= 2.0.0"
requires "jester#baca3f"
requires "karax#5cf360c"
"#).unwrap();
    let deps = parse_nimble_requires(&tmp);
    assert_eq!(deps, vec!["jester", "karax"]);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn nim_parses_nimble_requires_multiline_comma_continuation() {
    // Comma-continuation across lines without parentheses — the form used
    // by nim-libp2p's nimble file where deps spill across several lines.
    let tmp = std::env::temp_dir().join("bw-test-nimble-parse-multiline");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("pkg.nimble"), r#"version = "1.0"
requires "nim >= 2.2.4",
  "nimcrypto >= 0.6.0", "bearssl >= 0.2.7",
  "chronicles >= 0.11.0",
  "chronos >= 4.2.2", "stew >= 0.4.2", "unittest2", "results",
  "serialization"
"#).unwrap();
    let deps = parse_nimble_requires(&tmp);
    assert!(deps.contains(&"nimcrypto".to_string()), "nimcrypto missing: {deps:?}");
    assert!(deps.contains(&"bearssl".to_string()), "bearssl missing: {deps:?}");
    assert!(deps.contains(&"chronicles".to_string()), "chronicles missing: {deps:?}");
    assert!(deps.contains(&"chronos".to_string()), "chronos missing: {deps:?}");
    assert!(deps.contains(&"stew".to_string()), "stew missing: {deps:?}");
    assert!(deps.contains(&"results".to_string()), "results missing: {deps:?}");
    assert!(deps.contains(&"serialization".to_string()), "serialization missing: {deps:?}");
    assert!(!deps.contains(&"nim".to_string()), "nim should be excluded: {deps:?}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn nim_parses_nimble_requires_paren_block() {
    let tmp = std::env::temp_dir().join("bw-test-nimble-parse-paren");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("pkg.nimble"), r#"requires(
  "foo >= 1.0",
  "bar",
)"#).unwrap();
    let deps = parse_nimble_requires(&tmp);
    assert!(deps.contains(&"foo".to_string()), "foo missing: {deps:?}");
    assert!(deps.contains(&"bar".to_string()), "bar missing: {deps:?}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

#[test]
fn nim_extract_imports_handles_group_form() {
    let mut out = std::collections::HashSet::new();
    extract_nim_imports(
        "import strutils\nimport std/strformat\nimport pkg/foo/[bar, baz]\nfrom os import getEnv\nimport other as O\n",
        &mut out,
    );
    assert!(out.contains("strutils"));
    assert!(out.contains("std/strformat"));
    assert!(out.contains("pkg/foo/bar"));
    assert!(out.contains("pkg/foo/baz"));
    assert!(out.contains("os"));
    assert!(out.contains("other"));
}

#[test]
fn nim_module_to_path_tail_converts() {
    assert_eq!(nim_module_to_path_tail("strutils"), Some("strutils.nim".to_string()));
    assert_eq!(nim_module_to_path_tail("std/strutils"), Some("std/strutils.nim".to_string()));
}
