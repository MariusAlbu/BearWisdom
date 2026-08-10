// =============================================================================
// ecosystem/nuget/ — NuGet ecosystem (.NET: C#, F#, VB.NET)
//
// Demand-driven DLL metadata: rather than cracking every type from every
// declared package DLL up front (the old eager path that caused 71-minute
// indexing on aspnetcore), the ecosystem now:
//
//   1. `locate_roots` discovers one `ExternalDepRoot` per DLL.
//   2. `build_symbol_index` enumerates type names from each DLL's type-
//      definition table (cheap header scan via dotscope) and registers
//      `(module, TypeName) → ext:dotnet-type:<dll>!!<asm>!!<QualifiedType>`.
//   3. `uses_demand_driven_parse` returns `true`, skipping the eager dump.
//   4. The resolve engine's materialize-on-miss path cracks exactly the one
//      type a ref demands via `crack_one_dll_type`.
//
// Source files inside NuGet package dirs (contentFiles/cs/, src/) are still
// indexed when present; they win over DLL metadata on the same qnames.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

mod cs_header;
mod dll_locator;
mod dll_metadata;
mod manifest;
mod signature_format;
mod source_discovery;
mod symbol_index;
mod type_qname;

pub use dll_locator::nuget_packages_root;
pub use dll_metadata::parse_dotnet_externals;
pub use manifest::{
    implicit_usings_for_sdk, most_capable_sdk, parse_global_usings, parse_package_references,
    parse_package_references_full, parse_project_references, parse_sdk_type, DotnetSdkType,
    NuGetCoord, NuGetManifest,
};

pub(crate) use dll_metadata::crack_one_dll_type;
use dll_locator::locate_dlls_for_project;
pub(crate) use dll_metadata::list_dll_type_names;
use symbol_index::{build_nuget_source_symbol_index, resolve_nuget_source_symbols};

pub const ID: EcosystemId = EcosystemId::new("nuget");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["csharp", "fsharp", "vbnet"];
const LEGACY_ECOSYSTEM_TAG: &str = "dotnet";

pub struct NugetEcosystem;

impl Ecosystem for NugetEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }
    fn manifest_specs(&self) -> &'static [ManifestSpec] {
        MANIFESTS
    }

    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] {
        &[
            (".csproj", "dotnet"),
            (".fsproj", "dotnet"),
            (".vbproj", "dotnet"),
        ]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["bin", "obj", ".nuget"]
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        dll_roots_for_project(ctx.project_root)
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    /// Build `(module, TypeName) → ext:dotnet-type virtual path` by scanning
    /// the ECMA-335 type-definition table of each DLL (header-only, no method
    /// body parse). The virtual path is decoded by the materialize path to
    /// crack exactly that one type on demand.
    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        let mut index = SymbolLocationIndex::new();
        // DLL-backed type entries keyed by virtual path.
        for dep in dep_roots {
            for (simple_name, virt_path) in list_dll_type_names(&dep.root, &dep.module_path) {
                index.insert(
                    dep.module_path.clone(),
                    simple_name,
                    PathBuf::from(&virt_path),
                );
            }
        }
        // Source files in NuGet package dirs win over DLL metadata.
        index.extend(build_nuget_source_symbol_index(dep_roots));
        index
    }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_nuget_source_symbols(dep, symbols)
    }

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        let short = fqn.rsplit('.').next().unwrap_or(fqn);
        resolve_nuget_source_symbols(dep, &[short])
    }
}

impl ExternalSourceLocator for NugetEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        dll_roots_for_project(project_root)
    }

    // parse_metadata_only returns None: the demand-driven path handles all
    // DLL metadata extraction. Returning None here prevents the eager full-DLL
    // dump that previously wedged the indexer on large .NET solutions.
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<NugetEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NugetEcosystem)).clone()
}

/// Discover one `ExternalDepRoot` per DLL the project declares. Uses the same
/// project-file scan + NuGet cache probe as the old eager pass, but emits
/// only the DLL path — no symbols extracted yet.
fn dll_roots_for_project(project_root: &Path) -> Vec<ExternalDepRoot> {
    locate_dlls_for_project(project_root)
        .into_iter()
        .map(|(pkg_name, dll_path, _lang_id)| ExternalDepRoot {
            module_path: pkg_name,
            version: String::new(),
            root: dll_path,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

