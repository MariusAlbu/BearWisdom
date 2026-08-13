// =============================================================================
// ecosystem/dotnet_stdlib.rs — .NET shared framework (stdlib ecosystem)
//
// Probes `dotnet --info` or $DOTNET_ROOT for the shared-framework path
// (e.g. `C:/Program Files/dotnet/shared/Microsoft.NETCore.App/8.0.0/`).
// The DLLs there are reference assemblies for System.*, Microsoft.*
// namespaces — the .NET equivalent of what JdkSrc provides for Java.
//
// Demand-driven: `build_symbol_index` offers every public type name from
// those DLLs under the same `ext:dotnet-type:` virtual path NuGet mints, so a
// demanded type is cracked one type at a time through the shared materialize
// path. Activation: any CLR-family source present.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::{debug, warn};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, SymbolLocationIndex,
};
use crate::ecosystem::externals::{pick_newest_version, ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("dotnet-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "dotnet-stdlib";
const LANGUAGES: &[&str] = &["csharp", "fsharp", "vbnet"];

pub struct DotnetStdlibEcosystem;

impl Ecosystem for DotnetStdlibEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

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
        // .NET stdlib ships DLL metadata, not source. Types surface through
        // `build_symbol_index`; walk_root returns empty so no source walk is
        // attempted.
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn reset_demand_caches(&self) {
        super::nuget::flush_assembly_cache();
    }

    /// Offer `(module, TypeName) → ext:dotnet-type virtual path` for every
    /// public type in the framework's DLLs, in the exact encoding NuGet mints,
    /// so a demanded type materializes through the same per-type crack.
    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        let mut index = SymbolLocationIndex::new();
        for dep in dep_roots {
            let mut dlls: Vec<PathBuf> = Vec::new();
            collect_dlls(&dep.root, &mut dlls);
            // `read_dir` has no defined order and the index is first-writer-wins
            // on `(module, name)`, so the scan order decides which assembly owns
            // a name two of them both declare.
            dlls.sort();
            for dll in &dlls {
                for (name, virt_path) in super::nuget::list_dll_type_names(dll, &dep.module_path) {
                    index.insert(dep.module_path.clone(), name, PathBuf::from(&virt_path));
                }
            }
        }
        index
    }
}

impl ExternalSourceLocator for DotnetStdlibEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_dotnet_stdlib()
    }

    // parse_metadata_only returns None (trait default): every framework type is
    // offered through `build_symbol_index` and cracked only when demanded.
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
        if p.is_dir() {
            return Some(p);
        }
    }
    let dotnet_root = probe_dotnet_root()?;
    let shared = dotnet_root.join("shared").join("Microsoft.NETCore.App");
    if !shared.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(&shared).ok()?;
    let versions: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    pick_framework_version(&versions)
}

/// The highest installed shared-framework directory. Version segments compare
/// numerically, so `10.0.8` outranks `8.0.7` and `6.0.32` outranks `6.0.16`.
///
/// Nothing at this layer carries the project's target framework, so with more
/// than one install on the machine the pick is speculative and says so.
fn pick_framework_version(version_dirs: &[PathBuf]) -> Option<PathBuf> {
    let names: Vec<String> = version_dirs
        .iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .map(str::to_string)
        .collect();
    let newest = pick_newest_version(&names)?;
    if names.len() > 1 {
        warn!(
            "dotnet-stdlib: {} shared frameworks installed and no target-framework signal; \
             resolving against {}",
            names.len(),
            newest
        );
    }
    version_dirs
        .iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(newest.as_str()))
        .cloned()
}

fn probe_dotnet_root() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("DOTNET_ROOT") {
        let p = PathBuf::from(val);
        if p.is_dir() {
            return Some(p);
        }
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
                        if sdk_parent.is_dir() {
                            return Some(sdk_parent.to_path_buf());
                        }
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
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn collect_dlls(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".dll") {
            continue;
        }
        // Skip native / runtime DLLs that don't carry managed metadata.
        if name.starts_with("api-ms-")
            || name.starts_with("Microsoft.DiaSymReader")
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
    LOCATOR
        .get_or_init(|| Arc::new(DotnetStdlibEcosystem))
        .clone()
}

#[cfg(test)]
#[path = "dotnet_stdlib_tests.rs"]
mod tests;
