use super::*;

#[test]
fn ecosystem_identity() {
    let e = RobotBrowserEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert!(Ecosystem::languages(&e).contains(&"robot"));
}

#[test]
fn uses_demand_driven() {
    assert!(RobotBrowserEcosystem.uses_demand_driven_parse());
}

#[test]
fn non_browser_project_returns_empty_roots() {
    let dir = tempfile::tempdir().expect("tempdir");
    let roots = ExternalSourceLocator::locate_roots(&RobotBrowserEcosystem, dir.path());
    assert!(roots.is_empty(), "locate_roots must return empty when no Browser import found");
}

#[test]
fn browser_project_returns_root() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let robot_file = dir.path().join("suite.robot");
    std::fs::File::create(&robot_file)
        .unwrap()
        .write_all(b"*** Settings ***\nLibrary    Browser    retry_assertions_for=2 sec\n\n*** Test Cases ***\nOpen Page\n    New Browser\n")
        .unwrap();
    let roots = ExternalSourceLocator::locate_roots(&RobotBrowserEcosystem, dir.path());
    assert_eq!(roots.len(), 1, "locate_roots must return sentinel for Browser projects");
}

#[test]
fn browserstack_not_detected_as_browser() {
    // `BrowserStack` must NOT activate the Browser ecosystem.
    assert!(
        !is_browser_library_line("Library    BrowserStack"),
        "BrowserStack must not be treated as Browser library"
    );
}

#[test]
fn browser_library_line_with_args_detected() {
    assert!(
        is_browser_library_line("Library    Browser    retry_assertions_for=2 sec"),
        "Browser with args must be detected"
    );
    assert!(
        is_browser_library_line("Library     Browser"),
        "Browser with extra spaces must be detected"
    );
}

#[test]
fn path_browser_not_detected() {
    // `Library    path/to/browser.py` must NOT match.
    assert!(
        !is_browser_library_line("Library    path/to/browser.py"),
        "path-based browser import must not activate Browser ecosystem"
    );
}

#[test]
fn synthesize_contains_top_unresolved_keywords() {
    let files = synthesize_library();
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();

    // These are the actual top unresolved refs from robot-browser index.
    for kw in [
        "Set Browser Timeout",
        "New Browser",
        "Click With Options",
        "Select Options By",
        "Get Url",
        "Wait For Elements State",
        "Wait For",
        "Get Style",
        "Get Browser Catalog",
        "Promise To",
        "Close Page",
        "Add Cookie",
        "Evaluate JavaScript",
        "Type Text",
        "Get BoundingBox",
        "Set Retry Assertions For",
        "Get Property",
        "Mouse Button",
        "Get Checkbox State",
        "SessionStorage Get Item",
        "LocalStorage Get Item",
        "Set Assertion Formatters",
        "Log All Scopes",
        "Highlight Elements",
        "HTTP",
        "Get Element By",
        "Get Page Ids",
        "Scroll To",
        "New Persistent Context",
        "Connect To Browser",
        "Strict Mode Should Be",
        "Start Coverage",
        "SessionStorage Set Item",
        "Press Keys",
        "Merge Coverage Reports",
        "Fill Text",
        "New Context",
        "Take Screenshot",
        "Wait For Load State",
        "Click",
        "New Page",
    ] {
        assert!(names.contains(&kw), "Browser keyword `{kw}` must be synthesized");
    }
}

#[test]
fn all_symbols_are_function_kind() {
    let files = synthesize_library();
    for f in &files {
        for sym in &f.symbols {
            assert_eq!(
                sym.kind,
                crate::types::SymbolKind::Function,
                "Browser keyword `{}` should be Function kind",
                sym.name
            );
        }
    }
}

#[test]
fn parallel_vecs_consistent() {
    let files = synthesize_library();
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
fn each_file_has_exactly_one_symbol() {
    let files = synthesize_library();
    for f in &files {
        assert_eq!(f.symbols.len(), 1, "synthetic file {} should have exactly one symbol", f.path);
    }
}
