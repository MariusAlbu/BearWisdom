use super::*;

#[test]
fn ecosystem_identity() {
    let e = EmberHandlebarsHelpersEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["handlebars"]);
}

#[test]
fn activation_is_language_present() {
    let e = EmberHandlebarsHelpersEcosystem;
    assert!(matches!(
        Ecosystem::activation(&e),
        EcosystemActivation::LanguagePresent("handlebars")
    ));
}

#[test]
fn synthesizes_known_helpers() {
    let pf = synthesize_file();
    let names: std::collections::HashSet<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in [
        "eq", "not", "or", "and", "mut", "fn", "hash", "array", "concat",
        "if", "unless", "each", "did_insert", "did_update", "svg_jar",
        "moment_format", "format_number", "on", "on_key", "unique_id",
    ] {
        assert!(
            names.contains(expected),
            "ember-handlebars-helpers should synthesize `{expected}`; got {} symbols",
            pf.symbols.len()
        );
    }
}

#[test]
fn synthetic_file_uses_typescript_language_for_npm_globals_lookup() {
    // Symbols are stored as typescript-flavored under `__npm_globals__.<name>`
    // qualified-names so the TS resolver's bare-name fallback finds them
    // without needing an explicit import in each Handlebars-embedded JS region.
    let pf = synthesize_file();
    assert_eq!(pf.language, "typescript");
    assert!(pf.path.starts_with("ext:ember-handlebars-helpers:"));
    let svg_jar = pf.symbols.iter().find(|s| s.name == "svg_jar").expect("svg_jar present");
    assert_eq!(svg_jar.qualified_name, "__npm_globals__.svg_jar");
}

#[test]
fn helpers_have_function_kind() {
    let pf = synthesize_file();
    assert!(
        pf.symbols.iter().all(|s| s.kind == SymbolKind::Function),
        "all helpers should be Function kind"
    );
}

#[test]
fn locate_roots_skips_projects_without_hbs_files() {
    let tmp = std::env::temp_dir().join(format!(
        "bw-ember-test-empty-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let e = EmberHandlebarsHelpersEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&e, &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(roots.is_empty(), "no .hbs file → no synthetic dep root");
}

#[test]
fn locate_roots_activates_when_hbs_file_present() {
    let tmp = std::env::temp_dir().join(format!(
        "bw-ember-test-active-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("template.hbs"), "{{greeting}}").unwrap();
    let e = EmberHandlebarsHelpersEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&e, &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(roots.len(), 1, ".hbs file should activate synthetic");
    assert_eq!(roots[0].ecosystem, TAG);
}
