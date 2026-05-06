// =============================================================================
// ecosystem/perl_stdlib.rs — Perl core modules (stdlib ecosystem)
//
// Walks `<perl_root>/lib/<ver>/` for the Perl core .pm modules — Carp,
// Data::Dumper, File::Path, IO::File, Storable, Getopt::Long, etc. The
// perl extractor parses .pm directly so the standard walker pipeline
// emits real symbols.
//
// Note: Perl's *built-in functions* (`print`, `chomp`, `split`, `map`,
// `grep`, `keys`, ...) live inside the perl interpreter's C source and
// are NOT walkable here. Resolution of those bare-name builtins still
// depends on a hardcoded predicate (or accepts loss when the predicate
// is dropped).
//
// Probe order:
//   1. $BEARWISDOM_PERL_STDLIB — explicit dir override.
//   2. `perl -V:installprivlib` — returns the core lib dir
//      (`/usr/lib/x86_64-linux-gnu/perl/5.36`,
//      `C:/Strawberry/perl/lib`, etc.).
//   3. Standard install paths on each OS.
//
// Activation: `LanguagePresent("perl")` — every Perl project uses these
// core modules unconditionally (substrate per the trait doc).
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

pub const ID: EcosystemId = EcosystemId::new("perl-stdlib");
const TAG: &str = "perl-stdlib";
const LANGUAGES: &[&str] = &["perl"];

pub struct PerlStdlibEcosystem;

impl Ecosystem for PerlStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("perl")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_perl_stdlib()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_perl_tree(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for PerlStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_perl_stdlib()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_perl_tree(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PerlStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PerlStdlibEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_perl_stdlib() -> Vec<ExternalDepRoot> {
    let Some(dir) = probe_perl_lib() else {
        debug!("perl-stdlib: no installprivlib probed");
        return Vec::new();
    };
    debug!("perl-stdlib: using {}", dir.display());
    vec![ExternalDepRoot {
        module_path: "perl-stdlib".to_string(),
        version: String::new(),
        root: dir,
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_perl_lib() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_PERL_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    if let Some(p) = probe_via_perl_v() {
        return Some(p);
    }
    probe_standard_perl_paths()
}

fn probe_via_perl_v() -> Option<PathBuf> {
    // `perl -V:installprivlib` prints `installprivlib='<path>';` on stdout.
    // Robust across Linux distros and Strawberry/ActivePerl on Windows.
    let output = Command::new("perl").arg("-V:installprivlib").output().ok()?;
    if !output.status.success() { return None; }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("installprivlib='") {
            // Strip the trailing `';` (sometimes plus extra whitespace).
            let path = rest.trim_end_matches(';').trim_end_matches('\'');
            let p = PathBuf::from(path);
            if p.is_dir() { return Some(p); }
        }
    }
    None
}

fn probe_standard_perl_paths() -> Option<PathBuf> {
    // Common perl-core lib locations across distros + Windows.
    for candidate in [
        // Strawberry / ActivePerl on Windows
        "C:/Strawberry/perl/lib",
        "C:/Perl64/lib",
        "C:/Perl/lib",
        // macOS system perl
        "/System/Library/Perl/Extras",
        "/usr/local/Cellar/perl",
        // Linux distros (fall through to versioned subdir search)
        "/usr/lib/x86_64-linux-gnu/perl-base",
        "/usr/share/perl",
        "/usr/local/lib/perl5",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_dir() { return Some(p); }
        // Try with versioned subdir (e.g. /usr/share/perl/5.36/)
        let parent = PathBuf::from(candidate);
        if parent.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&parent) {
                let mut versioned: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir() && looks_like_perl_version(p))
                    .collect();
                versioned.sort();
                if let Some(latest) = versioned.into_iter().next_back() {
                    return Some(latest);
                }
            }
        }
    }
    None
}

fn looks_like_perl_version(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    // Match `5.30`, `5.36.0`, `5.40` — perl version stamp.
    name.starts_with("5.") && name.chars().nth(2).map(|c| c.is_ascii_digit()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_perl_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    // Perl module trees go ~6 levels deep (`Net/HTTP/NB.pm`,
    // `Mojolicious/Plugin/RenderFile.pm`). Cap at 12 for safety.
    if depth >= 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip test fixtures and arch-specific binary dirs that
                // don't contribute user-callable Perl source.
                if matches!(name, "t" | "test" | "tests" | "auto" | "unicore") {
                    continue;
                }
                if name.starts_with('.') { continue }
            }
            walk_dir(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".pm") { continue }
            let display = path.to_string_lossy().replace('\\', "/");
            out.push(WalkedFile {
                relative_path: format!("ext:perl-stdlib:{display}"),
                absolute_path: path,
                language: "perl",
            });
        }
    }
}

#[cfg(test)]
#[path = "perl_stdlib_tests.rs"]
mod tests;
