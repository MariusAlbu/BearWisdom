use super::*;

#[test]
fn language_id_matches_spec() {
    assert_eq!(Language::CSharp.language_id(), "csharp");
    assert_eq!(Language::TypeScript.language_id(), "typescript");
    assert_eq!(Language::Rust.language_id(), "rust");
    assert_eq!(Language::Python.language_id(), "python");
    assert_eq!(Language::Go.language_id(), "go");
    assert_eq!(Language::Java.language_id(), "java");
    assert_eq!(Language::Cpp.language_id(), "cpp");
}

#[test]
fn from_extension_known() {
    assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
    assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
    assert_eq!(Language::from_extension("cs"), Some(Language::CSharp));
    assert_eq!(Language::from_extension("py"), Some(Language::Python));
    assert_eq!(Language::from_extension("go"), Some(Language::Go));
    assert_eq!(Language::from_extension("java"), Some(Language::Java));
    assert_eq!(Language::from_extension("cpp"), Some(Language::Cpp));
    assert_eq!(Language::from_extension("hpp"), Some(Language::Cpp));
    assert_eq!(Language::from_extension("c"), Some(Language::Cpp));
}

#[test]
fn from_extension_unknown_returns_none() {
    assert_eq!(Language::from_extension("json"), None);
    assert_eq!(Language::from_extension("md"), None);
    assert_eq!(Language::from_extension("toml"), None);
}

#[test]
fn position_serde_roundtrip() {
    let p = Position { line: 10, character: 5 };
    let json = serde_json::to_string(&p).unwrap();
    let p2: Position = serde_json::from_str(&json).unwrap();
    assert_eq!(p, p2);
    // Verify camelCase field name
    assert!(json.contains("character"));
}
