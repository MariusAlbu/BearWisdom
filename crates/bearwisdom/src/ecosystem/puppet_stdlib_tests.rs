// Sibling test file for `puppet_stdlib.rs`.

use super::*;

#[test]
fn ecosystem_identity() {
    let e = PuppetStdlibEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["puppet"]);
}

#[test]
fn activation_is_language_present() {
    let e = PuppetStdlibEcosystem;
    assert!(matches!(
        Ecosystem::activation(&e),
        EcosystemActivation::LanguagePresent("puppet")
    ));
}

#[test]
fn uses_demand_driven() {
    assert!(PuppetStdlibEcosystem.uses_demand_driven_parse());
}

#[test]
fn synthesise_covers_all_builtins() {
    let files = synthesise_stdlib();
    // One file per *distinct* built-in name. BUILTIN_TYPES and
    // BUILTIN_FUNCTIONS overlap on names that are both core Puppet
    // built-ins and re-exported by puppetlabs-stdlib (`dig`, `merge`,
    // …), so the synthesise loop dedups them.
    let mut distinct: std::collections::HashSet<&'static str> =
        std::collections::HashSet::new();
    distinct.extend(BUILTIN_TYPES.iter().copied());
    distinct.extend(BUILTIN_FUNCTIONS.iter().copied());
    assert_eq!(files.len(), distinct.len());
}

#[test]
fn types_are_class_kind() {
    let files = synthesise_stdlib();
    for f in &files {
        if f.path.contains("/types/") {
            assert_eq!(f.symbols.len(), 1);
            assert_eq!(f.symbols[0].kind, SymbolKind::Class, "type {} should be Class", f.symbols[0].name);
        }
    }
}

#[test]
fn functions_are_function_kind() {
    let files = synthesise_stdlib();
    for f in &files {
        if f.path.contains("/functions/") {
            assert_eq!(f.symbols.len(), 1);
            assert_eq!(f.symbols[0].kind, SymbolKind::Function, "function {} should be Function", f.symbols[0].name);
        }
    }
}

#[test]
fn symbol_index_covers_all_builtins() {
    let e = PuppetStdlibEcosystem;
    let index = e.build_symbol_index(&[]);
    // Check a sample from each category.
    assert!(index.locate("puppet-stdlib", "file").is_some());
    assert!(index.locate("puppet-stdlib", "service").is_some());
    assert!(index.locate("puppet-stdlib", "include").is_some());
    assert!(index.locate("puppet-stdlib", "lookup").is_some());
}

#[test]
fn parse_metadata_only_returns_stdlib() {
    let e = PuppetStdlibEcosystem;
    let sentinel = ExternalDepRoot {
        module_path: "puppet-stdlib".into(),
        version: String::new(),
        root: PathBuf::from("ext:puppet-stdlib"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = Ecosystem::parse_metadata_only(&e, &sentinel).unwrap();
    assert!(!files.is_empty());
    let names: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();
    assert!(names.contains(&"file"));
    assert!(names.contains(&"include"));
    assert!(names.contains(&"lookup"));
}
