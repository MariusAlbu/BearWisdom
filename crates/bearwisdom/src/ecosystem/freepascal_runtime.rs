// =============================================================================
// ecosystem/freepascal_runtime.rs — Lazarus IDE + Free Pascal stdlib
//
// Probes a Lazarus install and surfaces three on-disk source trees as
// external dep roots:
//
//   - <root>/lcl/         — LCL (`TForm`, `TButton`, `TMenuItem`, etc.)
//   - <root>/components/  — Lazarus-bundled components (`codetools`, `chmhelp`)
//   - <root>/fpc/<ver>/source/{rtl,packages}/ — Free Pascal RTL + FCL
//                                              (`SysUtils`, `Classes`, `Math`)
//
// Activation: any Pascal project (`.pas`/`.pp`/`.lpr`/`.lpi`/`.lpk`).
// Probes scoop/apps/lazarus/current/ first (Windows scoop convention),
// then $LAZARUS_DIR, then standard install paths on each platform.
//
// Demand-driven parsing
// ---------------------
// `uses_demand_driven_parse` is true: the eager walk is suppressed.
// `build_symbol_index` (in `fpc_fragment_index`) scans each `.pas`/`.pp`/
// `.inc` file for the unit name (first `unit <Name>;` declaration) and
// interface-section top-level declarations (`type`, `procedure`, `function`,
// `var`, `const` followed by an identifier), so the demand loop can locate
// the right source file for any runtime symbol without parsing the entire
// stdlib up front. FPC unit files splice their real declarations in via
// `{$I fragment.inc}` rather than declaring them inline, so `.inc` fragments
// are scanned as independent declaration sources — see
// `fpc_fragment_index.rs` for how a fragment's missing `unit`/`interface`
// wrapper is handled.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

#[path = "fpc_fragment_index.rs"]
mod fpc_fragment_index;
#[path = "freepascal_discovery.rs"]
mod freepascal_discovery;

pub(crate) use freepascal_discovery::*;

pub const ID: EcosystemId = EcosystemId::new("freepascal-runtime");
pub(crate) const LEGACY_ECOSYSTEM_TAG: &str = "freepascal-runtime";
const LANGUAGES: &[&str] = &["pascal"];

pub struct FreePascalRuntimeEcosystem;

impl Ecosystem for FreePascalRuntimeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("pascal")
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        // A runtime locator owns no project-side caches. This set feeds the
        // PROJECT workspace scan — content dirs (tests/, examples/) must
        // never appear here or they prune every project's same-named dirs.
        &[]
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_freepascal_roots()
    }

    // Demand-driven: no eager walk. `build_symbol_index` registers each
    // Pascal unit's name and its interface-section declarations so the
    // Stage 2 loop can pull exactly the files a project's `uses` clause
    // references, without parsing the full ~900-file Lazarus+FPC stdlib.
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        fpc_fragment_index::build_pascal_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for FreePascalRuntimeEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_freepascal_roots()
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<FreePascalRuntimeEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(FreePascalRuntimeEcosystem)).clone()
}

#[cfg(test)]
#[path = "freepascal_runtime_tests.rs"]
mod tests;
