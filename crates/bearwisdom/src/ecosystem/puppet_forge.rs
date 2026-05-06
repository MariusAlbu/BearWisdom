// =============================================================================
// ecosystem/puppet_forge.rs — Puppet Forge (package ecosystem)
//
// Discovers modules installed via librarian-puppet (Puppetfile) or r10k, plus
// modules published to the Forge declared in metadata.json. Walks manifests/,
// functions/, types/, and plans/ subdirectories for .pp files.
//
// Cache locations probed (in order):
//   1. BEARWISDOM_PUPPET_MODULES env var (override)
//   2. ~/.puppetlabs/puppet/modules/
//   3. /etc/puppetlabs/code/modules/
//   4. /etc/puppetlabs/code/environments/production/modules/
//   5. modules/ under project root
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("puppet-forge");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["puppet"];
const LEGACY_ECOSYSTEM_TAG: &str = "puppet-forge";

// =============================================================================
// Ecosystem impl
// =============================================================================

pub struct PuppetForgeEcosystem;

impl Ecosystem for PuppetForgeEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `metadata.json` and/or `Puppetfile`. A bare
        // directory of `.pp` files with no manifest can't be resolved
        // against external Puppet Forge coordinates, so dropping the
        // LanguagePresent shotgun is correct per the trait doc.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_puppet_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_puppet_module(dep)
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_puppet_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for PuppetForgeEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_puppet_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_puppet_module(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<PuppetForgeEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(PuppetForgeEcosystem)).clone()
}

// =============================================================================
// Discovery
// =============================================================================

pub fn discover_puppet_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = collect_declared_modules(project_root);
    if declared.is_empty() {
        debug!("puppet-forge: no declared modules found");
        return Vec::new();
    }

    let search_dirs = modules_search_dirs(project_root);
    let mut roots = Vec::new();

    for dep_name in &declared {
        // Forge module names use author-module or author/module notation.
        // On disk they are stored as author-module or just module depending on
        // the installer. Normalise to the bare module name (the part after the
        // last `-` or `/`) for directory matching, but also try the full slug.
        let bare = dep_name
            .split(|c| c == '-' || c == '/')
            .last()
            .unwrap_or(dep_name.as_str());
        let slug = dep_name.replace('/', "-");

        for dir in &search_dirs {
            // Try `author-module` slug first, then bare module name.
            for candidate_name in [slug.as_str(), bare] {
                let candidate = dir.join(candidate_name);
                if candidate.is_dir() {
                    let version = read_module_version(&candidate);
                    roots.push(ExternalDepRoot {
                        module_path: dep_name.clone(),
                        version,
                        root: candidate,
                        ecosystem: LEGACY_ECOSYSTEM_TAG,
                        package_id: None,
                        requested_imports: Vec::new(),
                    });
                    break;
                }
            }
        }
    }

    debug!("puppet-forge: {} external module roots", roots.len());
    roots
}

/// Read all declared module names from metadata.json and/or Puppetfile.
pub fn collect_declared_modules(project_root: &Path) -> Vec<String> {
    let mut deps: Vec<String> = Vec::new();

    // --- metadata.json ---
    let meta = project_root.join("metadata.json");
    if let Ok(content) = std::fs::read_to_string(&meta) {
        parse_metadata_json_deps(&content, &mut deps);
    }

    // --- Puppetfile ---
    let puppetfile = project_root.join("Puppetfile");
    if let Ok(content) = std::fs::read_to_string(&puppetfile) {
        parse_puppetfile_deps(&content, &mut deps);
    }

    deps.sort();
    deps.dedup();
    deps
}

/// Parse `dependencies` array from a metadata.json string.
fn parse_metadata_json_deps(content: &str, out: &mut Vec<String>) {
    // Avoid a JSON parser dep — use a targeted string scan. Format:
    // "dependencies": [ {"name": "puppetlabs-stdlib", ...}, ... ]
    let mut in_deps = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("\"dependencies\"") { in_deps = true; }
        if in_deps {
            // Look for "name": "author-module"
            if let Some(name) = extract_json_string_field(trimmed, "name") {
                if name.contains('-') || name.contains('/') {
                    out.push(name);
                }
            }
            // End of dependencies array
            if trimmed == "]" || trimmed == "]," { in_deps = false; }
        }
    }
}

/// Extract a simple `"key": "value"` string from a JSON line.
fn extract_json_string_field<'a>(line: &'a str, key: &str) -> Option<String> {
    let search = format!("\"{key}\"");
    let key_pos = line.find(search.as_str())?;
    let after = &line[key_pos + search.len()..];
    let colon_pos = after.find(':')?;
    let after_colon = after[colon_pos + 1..].trim();
    if !after_colon.starts_with('"') { return None; }
    let inner = &after_colon[1..];
    let end = inner.find('"')?;
    let val = inner[..end].trim().to_string();
    if val.is_empty() { None } else { Some(val) }
}

/// Parse `mod 'author/module'` or `mod 'author-module'` lines from a Puppetfile.
fn parse_puppetfile_deps(content: &str, out: &mut Vec<String>) {
    for line in content.lines() {
        let trimmed = line.trim();
        // Skip comments and blank lines.
        if trimmed.is_empty() || trimmed.starts_with('#') { continue }
        // `mod 'puppetlabs/stdlib', '>=4.13.1'` or `mod "author-module"`
        let rest = match trimmed
            .strip_prefix("mod ")
            .or_else(|| trimmed.strip_prefix("mod\t"))
        {
            Some(r) => r.trim(),
            None => continue,
        };
        let name = extract_quoted_string(rest);
        if !name.is_empty() && (name.contains('-') || name.contains('/')) {
            out.push(name);
        }
    }
}

/// Extract the first single- or double-quoted string from `s`.
fn extract_quoted_string(s: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(start) = s.find(quote) {
            let inner = &s[start + 1..];
            if let Some(end) = inner.find(quote) {
                return inner[..end].to_string();
            }
        }
    }
    String::new()
}

/// Ordered list of directories to search for installed Puppet modules.
fn modules_search_dirs(project_root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Some(explicit) = std::env::var_os("BEARWISDOM_PUPPET_MODULES") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { dirs.push(p); }
    }

    if let Some(home) = dirs::home_dir() {
        let p = home.join(".puppetlabs").join("puppet").join("modules");
        if p.is_dir() { dirs.push(p); }
    }

    for fixed in [
        "/etc/puppetlabs/code/modules",
        "/etc/puppetlabs/code/environments/production/modules",
    ] {
        let p = PathBuf::from(fixed);
        if p.is_dir() { dirs.push(p); }
    }

    // Project-local modules/ dir (common for r10k environments).
    let local = project_root.join("modules");
    if local.is_dir() { dirs.push(local); }

    dirs
}

/// Read the version from a module's metadata.json, fall back to empty string.
fn read_module_version(module_root: &Path) -> String {
    let meta = module_root.join("metadata.json");
    let Ok(content) = std::fs::read_to_string(&meta) else { return String::new() };
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(ver) = extract_json_string_field(trimmed, "version") {
            return ver;
        }
    }
    String::new()
}

// =============================================================================
// Walk
// =============================================================================

/// Subdirectories within a Puppet module that contain .pp source files.
const PUPPET_SOURCE_DIRS: &[&str] = &["manifests", "functions", "types", "plans"];

fn walk_puppet_module(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    for sub in PUPPET_SOURCE_DIRS {
        let dir = dep.root.join(sub);
        if !dir.is_dir() { continue }
        walk_dir_bounded(&dir, &dep.root, dep, &mut out, 0);
    }
    out
}

fn walk_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".pp") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:puppet:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "puppet",
            });
        }
    }
}

// =============================================================================
// Symbol-location index (demand-driven pipeline entry)
// =============================================================================

fn build_puppet_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut index = SymbolLocationIndex::new();
    for dep in dep_roots {
        for wf in walk_puppet_module(dep) {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else { continue };
            for name in scan_puppet_header(&src) {
                index.insert(dep.module_path.clone(), name, wf.absolute_path.clone());
            }
        }
    }
    index
}

/// Line-based scan for top-level Puppet declarations: class, define, function,
/// type alias, plan. Returns unqualified names (e.g. `"apache"`, `"config"`).
pub(crate) fn scan_puppet_header(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') { continue }
        for kw in &["class ", "define ", "function ", "type ", "plan "] {
            if let Some(rest) = trimmed.strip_prefix(kw) {
                let name = rest
                    .split(|c: char| c == '(' || c == '{' || c == ' ' || c == '\t')
                    .next()
                    .unwrap_or("")
                    .trim();
                // Qualified name like `puppetlabs::stdlib::foo` — use the bare tail.
                let bare = name.split("::").last().unwrap_or(name);
                if !bare.is_empty() && bare.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    out.push(bare.to_string());
                }
                break;
            }
        }
    }
    out
}

// =============================================================================
// Manifest reader (metadata.json + Puppetfile)
// =============================================================================

/// Reads `metadata.json` and `Puppetfile` to surface declared forge modules
/// in `ProjectContext.manifests[ManifestKind::Puppet]`. Dependency names are
/// stored as bare module names (`"stdlib"`, `"apache"`, `"systemd"`) — the
/// shape the Puppet resolver compares against the prefix of qualified refs
/// like `apache::vhost`.
pub struct PuppetMetadataManifest;

impl ManifestReader for PuppetMetadataManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Puppet }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let slugs = collect_declared_modules(project_root);
        if slugs.is_empty() { return None }
        let mut data = ManifestData::default();
        for slug in slugs {
            let bare = slug
                .split(|c| c == '-' || c == '/')
                .last()
                .unwrap_or(slug.as_str())
                .to_string();
            if !bare.is_empty() {
                data.dependencies.insert(bare);
            }
        }
        Some(data)
    }
}

// =============================================================================
// Test wrappers
// =============================================================================

#[cfg(test)]
pub(super) fn _test_parse_metadata_json_deps(content: &str, out: &mut Vec<String>) {
    parse_metadata_json_deps(content, out);
}

#[cfg(test)]
pub(super) fn _test_parse_puppetfile_deps(content: &str, out: &mut Vec<String>) {
    parse_puppetfile_deps(content, out);
}

#[cfg(test)]
pub(super) fn _test_extract_quoted_string(s: &str) -> String {
    extract_quoted_string(s)
}

#[cfg(test)]
#[path = "puppet_forge_tests.rs"]
mod tests;
