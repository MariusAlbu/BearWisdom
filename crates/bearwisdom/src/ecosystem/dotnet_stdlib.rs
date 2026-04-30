// =============================================================================
// ecosystem/dotnet_stdlib.rs — .NET shared framework (stdlib ecosystem)
//
// Probes `dotnet --info` or $DOTNET_ROOT for the shared-framework path
// (e.g. `C:/Program Files/dotnet/shared/Microsoft.NETCore.App/8.0.0/`).
// The DLLs there are reference assemblies for System.*, Microsoft.*
// namespaces — the .NET equivalent of what JdkSrc provides for Java.
//
// Synthesis reuses the same dotscope path the NuGet ecosystem uses for
// package DLLs. Activation: any .NET language source present.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("dotnet-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "dotnet-stdlib";
const LANGUAGES: &[&str] = &["csharp", "fsharp", "vbnet"];

pub struct DotnetStdlibEcosystem;

impl Ecosystem for DotnetStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        // PowerShell runs on .NET — every cmdlet is a .NET type and PS scripts
        // routinely reference BCL types unqualified (`class MyError : Exception`,
        // `[System.Collections.Hashtable]::new()`). Without dotnet-stdlib active,
        // those refs land in unresolved_refs even though dotscope can index the
        // exact assemblies they need. Treat .ps1/.psm1 presence as a trigger
        // alongside the source-language CLR families.
        EcosystemActivation::Any(&[
            EcosystemActivation::LanguagePresent("csharp"),
            EcosystemActivation::LanguagePresent("fsharp"),
            EcosystemActivation::LanguagePresent("vbnet"),
            EcosystemActivation::LanguagePresent("powershell"),
        ])
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_dotnet_stdlib()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        // .NET stdlib is metadata-only (DLLs), not source. The indexer's
        // metadata path is where the real work happens; walk_root returns
        // empty so no source walk is attempted.
        Vec::new()
    }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<crate::types::ParsedFile>> {
        // Defer to NuGet's existing DLL→ParsedFile helper. Skip if the
        // probe returned a non-dir for any reason.
        if !dep.root.is_dir() { return None; }
        let mut dlls: Vec<PathBuf> = Vec::new();
        collect_dlls(&dep.root, &mut dlls);
        if dlls.is_empty() { return None; }
        let mut out = Vec::new();
        for dll in dlls.iter().take(400) {
            let Some(stem) = dll.file_stem().and_then(|s| s.to_str()) else { continue };
            match super::nuget::parse_dotnet_dll_public(dll, stem, "csharp") {
                Ok(pf) => out.push(pf),
                Err(e) => debug!("dotnet-stdlib: skip {}: {}", stem, e),
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        _dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        // .NET stdlib uses DLL metadata via `parse_metadata_only` — the
        // symbols land as ParsedFile entries directly and are indexed by
        // the normal write pass. No source symbol index needed.
        crate::ecosystem::symbol_index::SymbolLocationIndex::new()
    }
}

impl ExternalSourceLocator for DotnetStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_dotnet_stdlib()
    }
    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<crate::types::ParsedFile>> {
        let roots = discover_dotnet_stdlib();
        let mut out = Vec::new();
        for r in roots {
            if let Some(parsed) = <Self as Ecosystem>::parse_metadata_only(self, &r) {
                out.extend(parsed);
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }
}

fn discover_dotnet_stdlib() -> Vec<ExternalDepRoot> {
    let Some(framework_dir) = probe_shared_framework_dir() else {
        debug!("dotnet-stdlib: no shared framework probed");
        return Vec::new();
    };
    debug!("dotnet-stdlib: using {}", framework_dir.display());
    vec![ExternalDepRoot {
        module_path: "Microsoft.NETCore.App".to_string(),
        version: String::new(),
        root: framework_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_shared_framework_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_DOTNET_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    let dotnet_root = probe_dotnet_root()?;
    let shared = dotnet_root.join("shared").join("Microsoft.NETCore.App");
    if !shared.is_dir() { return None; }
    let entries = std::fs::read_dir(&shared).ok()?;
    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    versions.sort();
    versions.into_iter().next_back()
}

fn probe_dotnet_root() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("DOTNET_ROOT") {
        let p = PathBuf::from(val);
        if p.is_dir() { return Some(p); }
    }
    // Ask `dotnet --info`.
    if let Ok(output) = Command::new("dotnet").arg("--info").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let trimmed = line.trim();
                // Match "Base Path:   /usr/share/dotnet/sdk/8.0.100/"
                if let Some(rest) = trimmed.strip_prefix("Base Path:") {
                    let p = PathBuf::from(rest.trim());
                    // Walk up to the dotnet root (parent of sdk/).
                    if let Some(sdk_parent) = p.parent().and_then(|p| p.parent()) {
                        if sdk_parent.is_dir() { return Some(sdk_parent.to_path_buf()); }
                    }
                }
            }
        }
    }
    // Common install paths.
    for candidate in [
        "C:/Program Files/dotnet",
        "C:/Program Files (x86)/dotnet",
        "/usr/share/dotnet",
        "/usr/local/share/dotnet",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_dir() { return Some(p); }
    }
    None
}

fn collect_dlls(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !name.ends_with(".dll") { continue }
        // Skip native / runtime DLLs that don't carry managed metadata.
        if name.starts_with("api-ms-") || name.starts_with("Microsoft.DiaSymReader")
            || name.ends_with(".Native.dll")
        {
            continue;
        }
        out.push(path);
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<DotnetStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(DotnetStdlibEcosystem)).clone()
}
