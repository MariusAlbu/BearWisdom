use std::sync::Arc;

use crate::ecosystem::{
    default_registry, Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind,
    EcosystemRegistry, LocateContext,
};
use crate::ecosystem::externals::ExternalDepRoot;

#[test]
fn default_registry_contains_package_ecosystems() {
    let ids: Vec<&str> = default_registry()
        .all()
        .iter()
        .map(|e| e.id().as_str())
        .collect();
    for expected in [
        "maven", "npm", "hex", "cargo", "pypi", "go-mod", "spm", "nuget", "pub", "rubygems",
        "cran", "composer", "cabal", "nimble", "cpan", "opam", "luarocks", "zig-pkg",
    ] {
        assert!(
            ids.contains(&expected),
            "ecosystem {expected} missing from default_registry; got {ids:?}",
        );
    }
}

#[test]
fn registry_lookup_by_id_and_language() {
    struct DummyEcosystem;
    impl Ecosystem for DummyEcosystem {
        fn id(&self) -> EcosystemId {
            EcosystemId::new("dummy")
        }
        fn kind(&self) -> EcosystemKind {
            EcosystemKind::Package
        }
        fn languages(&self) -> &'static [&'static str] {
            &["fake-lang"]
        }
        fn activation(&self) -> EcosystemActivation {
            EcosystemActivation::Never
        }
        fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
            Vec::new()
        }
    }

    let mut reg = EcosystemRegistry::new();
    reg.register(Arc::new(DummyEcosystem));

    assert!(reg.get(EcosystemId::new("dummy")).is_some());
    assert!(reg.get(EcosystemId::new("other")).is_none());
    assert_eq!(reg.for_language("fake-lang").len(), 1);
    assert_eq!(reg.for_language("real-lang").len(), 0);
}
