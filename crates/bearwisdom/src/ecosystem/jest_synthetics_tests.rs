use super::*;

#[test]
fn synthetic_globals_file_emits_test_runner_globals() {
    let file = synthesize_globals_file();
    let names: Vec<&str> = file.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &[
        "describe", "it", "test", "expect",
        "beforeEach", "afterEach", "beforeAll", "afterAll",
        "fit", "fdescribe", "xit", "xdescribe",
    ] {
        assert!(
            names.contains(expected),
            "missing test global '{expected}' in synthesized file"
        );
    }
}

#[test]
fn synthetic_globals_file_emits_jest_namespace_members() {
    let file = synthesize_globals_file();
    let qnames: Vec<&str> = file.symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    for expected in &[
        "jest",
        "jest.Mock", "jest.Mocked", "jest.MockedFunction", "jest.MockedClass",
        "jest.SpyInstance", "jest.fn", "jest.spyOn", "jest.clearAllMocks",
        "jest.useFakeTimers", "jest.advanceTimersByTime",
    ] {
        assert!(
            qnames.contains(expected),
            "missing jest namespace member '{expected}'"
        );
    }
}

#[test]
fn synthetic_globals_file_emits_vi_namespace_alias() {
    let file = synthesize_globals_file();
    let qnames: Vec<&str> = file.symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    // Vitest mirrors jest at the namespace level — same members, `vi.*` qname.
    for expected in &["vi", "vi.fn", "vi.Mock", "vi.spyOn", "vi.useFakeTimers"] {
        assert!(
            qnames.contains(expected),
            "missing vi namespace member '{expected}'"
        );
    }
}

#[test]
fn synthetic_globals_file_emits_jsx_namespace_members() {
    let file = synthesize_globals_file();
    let qnames: Vec<&str> = file.symbols.iter().map(|s| s.qualified_name.as_str()).collect();
    for expected in &[
        "JSX", "JSX.Element", "JSX.IntrinsicElements", "JSX.IntrinsicAttributes",
        "JSX.ElementClass",
    ] {
        assert!(
            qnames.contains(expected),
            "missing JSX namespace member '{expected}'"
        );
    }
}

#[test]
fn synthetic_globals_path_is_external_prefixed() {
    let file = synthesize_globals_file();
    assert!(
        file.path.starts_with("ext:"),
        "synthetic file must use ext: prefix so the indexer routes it as external; got {}",
        file.path,
    );
}

#[test]
fn synthetic_globals_no_duplicate_qualified_names() {
    let file = synthesize_globals_file();
    let mut seen = std::collections::HashSet::new();
    for sym in &file.symbols {
        let unique = seen.insert(sym.qualified_name.clone());
        assert!(
            unique,
            "duplicate qualified_name in synthetic file: {}",
            sym.qualified_name
        );
    }
}
