// =============================================================================
// ecosystem/vba_typelibs.rs — VBA OLE typelibs (stdlib ecosystem, stub)
//
// VBA's "stdlib" is the set of Office/Windows COM type libraries
// (Microsoft Word Object Library, Excel, Outlook, Scripting Runtime,
// etc.). Indexing them requires OLE typelib introspection via
// `ITypeLib` + `ITypeInfo` Windows COM APIs or parsing `.tlb` files.
//
// This ecosystem is registered so the activation seam stays complete,
// but `locate_roots` returns empty today. Implementing the COM probe
// is deferred — it would require the `windows` crate's OLE bindings
// or a dedicated `.tlb` reader, neither of which is currently in the
// workspace. Until that arrives, VBA projects continue to rely on the
// hardcoded lists in `languages/vba/builtins.rs`.
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, Platform,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("vba-typelibs");
const LEGACY_ECOSYSTEM_TAG: &str = "vba-typelibs";
const LANGUAGES: &[&str] = &["vba"];

pub struct VbaTypelibsEcosystem;

impl Ecosystem for VbaTypelibsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::All(&[
            EcosystemActivation::AlwaysOnPlatform(Platform::Windows),
            EcosystemActivation::LanguagePresent("vba"),
        ])
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // TODO: OLE typelib introspection via `windows` crate COM bindings.
        Vec::new()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

impl ExternalSourceLocator for VbaTypelibsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> { Vec::new() }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<VbaTypelibsEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(VbaTypelibsEcosystem)).clone()
}
