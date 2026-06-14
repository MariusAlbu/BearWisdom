// Sibling tests for `vba_typelibs.rs`. VBA typelib introspection is not yet
// implemented, so the ecosystem is a well-typed no-op that opts into the
// demand-driven path with an empty symbol index.

use super::*;

#[test]
fn ecosystem_identity() {
    let eco = VbaTypelibsEcosystem;
    assert_eq!(eco.id().as_str(), "vba-typelibs");
    assert_eq!(eco.kind(), EcosystemKind::Stdlib);
    assert_eq!(eco.languages(), &["vba"]);
}

#[test]
fn uses_demand_driven_parse_is_true() {
    assert!(VbaTypelibsEcosystem.uses_demand_driven_parse());
}

#[test]
fn locate_roots_returns_empty() {
    use std::collections::HashMap;
    use std::path::PathBuf;
    let manifests: HashMap<EcosystemId, Vec<PathBuf>> = HashMap::new();
    let ctx = LocateContext {
        project_root: std::path::Path::new("."),
        manifests: &manifests,
        active_ecosystems: &[],
    };
    assert!(Ecosystem::locate_roots(&VbaTypelibsEcosystem, &ctx).is_empty());
}

#[test]
fn build_symbol_index_is_empty_when_no_roots() {
    let index = VbaTypelibsEcosystem.build_symbol_index(&[]);
    assert!(index.is_empty());
}
