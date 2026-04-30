use super::*;

#[test]
fn ecosystem_identity() {
    let e = RobotBuiltinEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert!(Ecosystem::languages(&e).contains(&"robot"));
}

#[test]
fn activation_is_language_present() {
    let e = RobotBuiltinEcosystem;
    assert!(matches!(
        Ecosystem::activation(&e),
        EcosystemActivation::LanguagePresent("robot")
    ));
}

#[test]
fn uses_demand_driven() {
    assert!(RobotBuiltinEcosystem.uses_demand_driven_parse());
}

#[test]
fn locate_roots_always_returns_sentinel() {
    use std::path::Path;
    let roots = ExternalSourceLocator::locate_roots(&RobotBuiltinEcosystem, Path::new("."));
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].ecosystem, "robot-builtin");
}

#[test]
fn synthesize_contains_builtin_keywords() {
    let files = synthesize_stdlib();
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();

    for kw in [
        "Log", "Set Variable", "Should Be Equal", "Run Keyword",
        "Sleep", "Fail", "Convert To String", "Create List",
    ] {
        assert!(names.contains(&kw), "BuiltIn keyword `{kw}` must be synthesized");
    }
}

#[test]
fn synthesize_contains_collections_keywords() {
    let files = synthesize_stdlib();
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();

    for kw in ["Log List", "Sort List", "Dictionary Should Contain Value", "Lists Should Be Equal"] {
        assert!(names.contains(&kw), "Collections keyword `{kw}` must be synthesized");
    }
}

#[test]
fn synthesize_contains_string_keywords() {
    let files = synthesize_stdlib();
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();

    for kw in ["Replace String", "Convert To Lower Case", "Split String", "Get Substring"] {
        assert!(names.contains(&kw), "String keyword `{kw}` must be synthesized");
    }
}

#[test]
fn synthesize_contains_os_keywords() {
    let files = synthesize_stdlib();
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();

    for kw in [
        "File Should Exist", "Remove File", "Set Environment Variable",
        "List Directory", "Create Directory",
    ] {
        assert!(names.contains(&kw), "OperatingSystem keyword `{kw}` must be synthesized");
    }
}

#[test]
fn synthesize_contains_datetime_keywords() {
    let files = synthesize_stdlib();
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();

    for kw in ["Get Current Date", "Subtract Date From Date", "Convert Date"] {
        assert!(names.contains(&kw), "DateTime keyword `{kw}` must be synthesized");
    }
}

#[test]
fn all_symbols_are_function_kind() {
    let files = synthesize_stdlib();
    for f in &files {
        for sym in &f.symbols {
            assert_eq!(
                sym.kind,
                crate::types::SymbolKind::Function,
                "robot stdlib keyword `{}` should be Function kind",
                sym.name
            );
        }
    }
}

#[test]
fn parallel_vecs_consistent() {
    let files = synthesize_stdlib();
    for f in &files {
        assert_eq!(
            f.symbols.len(),
            f.symbol_origin_languages.len(),
            "symbol_origin_languages must match symbols for {}",
            f.path
        );
        assert_eq!(
            f.symbols.len(),
            f.symbol_from_snippet.len(),
            "symbol_from_snippet must match symbols for {}",
            f.path
        );
    }
}

#[test]
fn parse_metadata_only_returns_stdlib() {
    let e = RobotBuiltinEcosystem;
    let root = synthetic_dep_root();
    let files = Ecosystem::parse_metadata_only(&e, &root).expect("must return Some");
    assert!(!files.is_empty());
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();
    assert!(names.contains(&"Log"));
    assert!(names.contains(&"Should Be Equal"));
    assert!(names.contains(&"Get Current Date"));
}

#[test]
fn each_file_has_exactly_one_symbol() {
    let files = synthesize_stdlib();
    for f in &files {
        assert_eq!(
            f.symbols.len(),
            1,
            "synthetic file {} should have exactly one symbol",
            f.path
        );
    }
}
