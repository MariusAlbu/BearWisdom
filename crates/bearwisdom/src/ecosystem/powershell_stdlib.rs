// =============================================================================
// ecosystem/powershell_stdlib.rs — PowerShell built-in modules (stdlib ecosystem)
//
// Probes `$PSHOME/Modules/` by running `pwsh -NoProfile -Command '$PSHOME'`.
//
// Fallback discovery order when `pwsh` is absent or fails:
//   1. $BEARWISDOM_PSHOME (explicit override)
//   2. C:/Program Files/PowerShell/7/ (Windows PS 7)
//   3. C:/Windows/System32/WindowsPowerShell/v1.0/ (Windows WPS 5.1)
//   4. /usr/local/microsoft/powershell/<latest>/ (Linux/macOS)
//   5. /usr/local/share/powershell/ (Linux/macOS alternate)
//
// Only walks `Microsoft.PowerShell.*` module trees. Other built-ins
// (e.g. PackageManagement, PowerShellGet) are intentionally excluded
// from the stdlib umbrella — they live in PSGallery.
//
// Activation: LanguagePresent("powershell").
// Uses demand-driven parse; build_symbol_index delegates to psgallery.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("powershell-stdlib");

const LEGACY_ECOSYSTEM_TAG: &str = "powershell-stdlib";
const LANGUAGES: &[&str] = &["powershell"];

/// Module name prefixes that belong to the PS built-in stdlib set.
/// Walk-limited to avoid indexing community modules that happen to sit in PSHOME.
const STDLIB_MODULE_PREFIXES: &[&str] = &[
    "Microsoft.PowerShell.Utility",
    "Microsoft.PowerShell.Management",
    "Microsoft.PowerShell.Security",
    "Microsoft.PowerShell.Core",
    "Microsoft.PowerShell.Host",
    "Microsoft.PowerShell.Diagnostics",
];

pub struct PowerShellStdlibEcosystem;

impl Ecosystem for PowerShellStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("powershell")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_powershell_stdlib()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::psgallery::walk_ps_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        super::psgallery::build_powershell_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for PowerShellStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_powershell_stdlib()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::psgallery::walk_ps_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PowerShellStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PowerShellStdlibEcosystem)).clone()
}

// ===========================================================================
// Discovery
// ===========================================================================

fn discover_powershell_stdlib() -> Vec<ExternalDepRoot> {
    let Some(pshome) = probe_pshome() else {
        debug!("powershell-stdlib: no PSHOME found; stdlib not indexed");
        return Vec::new();
    };

    let modules_dir = pshome.join("Modules");
    if !modules_dir.is_dir() {
        debug!("powershell-stdlib: PSHOME/Modules not found at {}", modules_dir.display());
        return Vec::new();
    }

    debug!("powershell-stdlib: walking {}", modules_dir.display());
    let mut roots = Vec::new();

    let Ok(entries) = std::fs::read_dir(&modules_dir) else { return Vec::new() };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !STDLIB_MODULE_PREFIXES.iter().any(|&prefix| name == prefix) {
            continue;
        }

        // A module dir may contain version subdirs (e.g. `7.0.0.0/`).
        let root = pick_module_root(&path).unwrap_or(path.clone());

        roots.push(ExternalDepRoot {
            module_path: name.to_string(),
            version: extract_version_from_path(&root),
            root,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }

    debug!("powershell-stdlib: {} stdlib module roots", roots.len());
    roots
}

/// Pick the "best" root inside a module dir — prefer the highest-version
/// numbered subdir; fall back to the module dir itself.
fn pick_module_root(module_dir: &Path) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(module_dir) else { return None };
    let mut versioned: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(|e| e.path())
        .collect();
    versioned.sort();
    versioned.into_iter().next_back()
}

fn extract_version_from_path(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .filter(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or("")
        .to_string()
}

// ===========================================================================
// PSHOME probe
// ===========================================================================

/// Probe for the PowerShell home directory.
///
/// Priority:
///   1. `$BEARWISDOM_PSHOME` env var (test / CI override).
///   2. Run `pwsh -NoProfile -Command $PSHOME` — definitive when pwsh is on PATH.
///   3. Well-known install locations (deterministic, no subprocess).
pub(crate) fn probe_pshome() -> Option<PathBuf> {
    // 1. Explicit override.
    if let Ok(explicit) = std::env::var("BEARWISDOM_PSHOME") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }

    // 2. Ask pwsh directly.
    if let Some(p) = ask_pwsh_pshome() {
        return Some(p);
    }

    // 3. Deterministic fallbacks.
    static_pshome_candidates()
        .into_iter()
        .find(|p| p.is_dir())
}

fn ask_pwsh_pshome() -> Option<PathBuf> {
    // Try `pwsh` first (PS 6+), then `powershell` (WPS 5.1 on Windows).
    for bin in &["pwsh", "powershell"] {
        let result = Command::new(bin)
            .args(["-NoProfile", "-NonInteractive", "-Command", "$PSHOME"])
            .output();
        let Ok(output) = result else { continue };
        if !output.status.success() {
            continue;
        }
        let s = String::from_utf8(output.stdout).ok()?;
        let trimmed = s.trim();
        if trimmed.is_empty() {
            continue;
        }
        let p = PathBuf::from(trimmed);
        if p.is_dir() {
            debug!("powershell-stdlib: PSHOME = {} (via {})", p.display(), bin);
            return Some(p);
        }
    }
    None
}

fn static_pshome_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        // Enumerate versioned dirs under Program Files\PowerShell (PS 7+).
        let mut cands: Vec<PathBuf> = Vec::new();
        let pf = std::env::var("ProgramFiles")
            .unwrap_or_else(|_| "C:/Program Files".to_string());
        let ps_base = PathBuf::from(&pf).join("PowerShell");
        if ps_base.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&ps_base) {
                let mut versioned: Vec<PathBuf> = entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter(|e| {
                        e.file_name()
                            .to_str()
                            .is_some_and(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    })
                    .map(|e| e.path())
                    .collect();
                versioned.sort();
                versioned.reverse(); // newest first
                cands.extend(versioned);
            }
        }
        cands.push(PathBuf::from("C:/Windows/System32/WindowsPowerShell/v1.0"));
        cands
    } else {
        // Linux / macOS — scan /usr/local/microsoft/powershell/<version>/
        let ms_base = PathBuf::from("/usr/local/microsoft/powershell");
        let mut cands: Vec<PathBuf> = Vec::new();
        if ms_base.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&ms_base) {
                let mut versioned: Vec<PathBuf> = entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.path())
                    .collect();
                versioned.sort();
                versioned.reverse();
                cands.extend(versioned);
            }
        }
        cands.push(PathBuf::from("/usr/local/share/powershell"));
        cands.push(PathBuf::from("/opt/microsoft/powershell/7"));
        cands
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let eco = PowerShellStdlibEcosystem;
        assert_eq!(eco.id(), ID);
        assert_eq!(Ecosystem::kind(&eco), EcosystemKind::Stdlib);
        assert_eq!(Ecosystem::languages(&eco), &["powershell"]);
    }

    #[test]
    fn legacy_locator_tag() {
        assert_eq!(ExternalSourceLocator::ecosystem(&PowerShellStdlibEcosystem), "powershell-stdlib");
    }

    #[test]
    fn activation_is_language_present() {
        let eco = PowerShellStdlibEcosystem;
        assert!(matches!(
            eco.activation(),
            EcosystemActivation::LanguagePresent("powershell")
        ));
    }

    #[test]
    fn stdlib_module_prefixes_are_non_empty() {
        assert!(!STDLIB_MODULE_PREFIXES.is_empty());
        for p in STDLIB_MODULE_PREFIXES {
            assert!(p.starts_with("Microsoft.PowerShell."), "unexpected prefix: {p}");
        }
    }

    #[test]
    fn probe_pshome_does_not_panic() {
        // Just verifies it returns Some(dir) or None without panicking.
        // On a machine without PS installed, returns None — that is correct.
        let _result = probe_pshome();
    }

    #[test]
    fn discover_returns_empty_for_pshome_without_modules_dir() {
        // When BEARWISDOM_PSHOME points to a dir that exists but has no
        // Modules/ subdir, locate_roots returns empty (PSHOME is found but
        // nothing to walk). This tests the Modules-not-found branch without
        // depending on whether pwsh is installed on the test machine.
        let tmp = std::env::temp_dir().join("bw-test-pshome-no-modules");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // tmp exists but has no Modules/ subdir.
        std::env::set_var("BEARWISDOM_PSHOME", tmp.to_str().unwrap());
        let roots = discover_powershell_stdlib();
        std::env::remove_var("BEARWISDOM_PSHOME");
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(roots.is_empty(), "expected empty roots for PSHOME without Modules/");
    }

    #[test]
    fn discover_finds_stdlib_modules_given_mock_pshome() {
        let tmp = std::env::temp_dir().join("bw-test-pshome-mock");
        let _ = std::fs::remove_dir_all(&tmp);
        // Build a minimal PSHOME/Modules/<name>/ structure.
        let utility_dir = tmp.join("Modules").join("Microsoft.PowerShell.Utility");
        std::fs::create_dir_all(&utility_dir).unwrap();
        // A .psm1 file directly in the module dir.
        std::fs::write(
            utility_dir.join("Microsoft.PowerShell.Utility.psm1"),
            "function Write-Host { }\n",
        )
        .unwrap();

        std::env::set_var("BEARWISDOM_PSHOME", tmp.to_str().unwrap());
        let roots = discover_powershell_stdlib();
        std::env::remove_var("BEARWISDOM_PSHOME");

        assert_eq!(roots.len(), 1, "expected 1 stdlib root, got {roots:?}");
        assert_eq!(roots[0].module_path, "Microsoft.PowerShell.Utility");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn walk_root_finds_psm1_files() {
        let tmp = std::env::temp_dir().join("bw-test-ps-walk-stdlib");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Microsoft.PowerShell.Utility.psm1"), "function Get-Date { }\n")
            .unwrap();
        std::fs::write(tmp.join("helper.ps1"), "function help { }\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "Microsoft.PowerShell.Utility".to_string(),
            version: "7.0.0.0".to_string(),
            root: tmp.clone(),
            ecosystem: "powershell-stdlib",
            package_id: None,
            requested_imports: Vec::new(),
        };

        let eco = PowerShellStdlibEcosystem;
        let files = Ecosystem::walk_root(&eco, &dep);
        assert_eq!(files.len(), 2, "expected 2 files, got {files:?}");
        assert!(files.iter().all(|f| f.language == "powershell"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn symbol_index_delegation_works() {
        let tmp = std::env::temp_dir().join("bw-test-ps-sym-index");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("Test.psm1"),
            "function Get-Item { }\nfunction Set-Item { }\nclass Item { }\n",
        )
        .unwrap();

        let dep = ExternalDepRoot {
            module_path: "Microsoft.PowerShell.Management".to_string(),
            version: String::new(),
            root: tmp.clone(),
            ecosystem: "powershell-stdlib",
            package_id: None,
            requested_imports: Vec::new(),
        };

        let eco = PowerShellStdlibEcosystem;
        let idx = eco.build_symbol_index(&[dep]);
        let found = idx.find_by_name("Get-Item");
        assert!(!found.is_empty(), "Get-Item not found in index");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
