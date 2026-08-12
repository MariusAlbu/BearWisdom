// =============================================================================
// indexer/demand_symbol_index_tests — unit tests for symbol-index assembly
// =============================================================================

use std::path::{Path, PathBuf};

use super::*;
use crate::ecosystem::{EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};

fn dep_root(ecosystem: &'static str, module_path: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module_path.to_string(),
        version: String::new(),
        root: PathBuf::from("/fake"),
        ecosystem,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// A single-language stdlib ecosystem — stands in for `freepascal-runtime`,
/// `rubygems`, etc. Its index must come back language-tagged.
struct FakeSingleLangEcosystem;
impl Ecosystem for FakeSingleLangEcosystem {
    fn id(&self) -> EcosystemId {
        EcosystemId::new("fake-single-lang")
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        &["pascal"]
    }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Never
    }
    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        Vec::new()
    }
    fn build_symbol_index(&self, _dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        let mut idx = SymbolLocationIndex::new();
        idx.insert("fpc-rtl-objpas", "SysUtils", "/fpc/rtl/objpas/sysutils.pp");
        idx
    }
}

/// A multi-language package ecosystem — stands in for Hex (elixir/erlang/
/// gleam), Maven, npm. Its index must come back untagged: per-file
/// extension/import detection is the correct dispatch when one ecosystem
/// spans several languages.
struct FakeMultiLangEcosystem;
impl Ecosystem for FakeMultiLangEcosystem {
    fn id(&self) -> EcosystemId {
        EcosystemId::new("fake-multi-lang")
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        &["elixir", "erlang", "gleam"]
    }
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Never
    }
    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        Vec::new()
    }
    fn build_symbol_index(&self, _dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        let mut idx = SymbolLocationIndex::new();
        idx.insert("deps/decimal", "new", "/deps/decimal/lib/decimal.ex");
        idx
    }
}

#[test]
fn single_language_ecosystem_index_gets_tagged() {
    let mut by_eco: HashMap<&'static str, Vec<ExternalDepRoot>> = HashMap::new();
    by_eco.insert("fake-single-lang", vec![dep_root("fake-single-lang", "fpc-rtl-objpas")]);
    let mut ecosystems: HashMap<&'static str, Arc<dyn Ecosystem>> = HashMap::new();
    ecosystems.insert("fake-single-lang", Arc::new(FakeSingleLangEcosystem));

    let idx = build_demand_symbol_index(&by_eco, &ecosystems);

    assert_eq!(
        idx.language_hint(Path::new("/fpc/rtl/objpas/sysutils.pp")),
        Some("pascal"),
        "a single-language ecosystem's pulled files must carry its language"
    );
}

#[test]
fn multi_language_ecosystem_index_stays_untagged() {
    let mut by_eco: HashMap<&'static str, Vec<ExternalDepRoot>> = HashMap::new();
    by_eco.insert("fake-multi-lang", vec![dep_root("fake-multi-lang", "deps/decimal")]);
    let mut ecosystems: HashMap<&'static str, Arc<dyn Ecosystem>> = HashMap::new();
    ecosystems.insert("fake-multi-lang", Arc::new(FakeMultiLangEcosystem));

    let idx = build_demand_symbol_index(&by_eco, &ecosystems);

    assert_eq!(
        idx.language_hint(Path::new("/deps/decimal/lib/decimal.ex")),
        None,
        "a multi-language ecosystem must not force a single language onto its files"
    );
}
