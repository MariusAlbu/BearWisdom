// =============================================================================
// ecosystem/rubygems.rs — RubyGems / Bundler ecosystem
//
// Phase 2 + 3: consolidates `indexer/externals/ruby.rs` +
// `indexer/manifest/gemfile.rs`. Prefers `Gemfile.lock` for exact
// version pinning; falls back to `Gemfile` declarations. Probes
// vendor/bundle, XDG user gem home, classic `~/.gem`, legacy
// `~/gems`, and `$GEM_HOME/gems/` for installed gems.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("rubygems");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["ruby"];
const LEGACY_ECOSYSTEM_TAG: &str = "ruby";

pub struct RubygemsEcosystem;

impl Ecosystem for RubygemsEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("ruby"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_ruby_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ruby_root(dep)
    }
}

impl ExternalSourceLocator for RubygemsEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ruby_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ruby_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<RubygemsEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(RubygemsEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (migrated from indexer/manifest/gemfile.rs)
// ===========================================================================

pub struct GemfileManifest;

impl ManifestReader for GemfileManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Gemfile }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let gemfile_path = project_root.join("Gemfile");
        if !gemfile_path.is_file() { return None }
        let content = std::fs::read_to_string(&gemfile_path).ok()?;
        let mut data = ManifestData::default();
        for name in parse_gemfile_gems(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_gemfile_gems(content: &str) -> Vec<String> {
    let mut gems = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') { continue }
        let rest = if let Some(r) = trimmed.strip_prefix("gem ") { r.trim() } else { continue };
        let name = if let Some(r) = rest.strip_prefix('\'') {
            r.split('\'').next().unwrap_or("").trim()
        } else if let Some(r) = rest.strip_prefix('"') {
            r.split('"').next().unwrap_or("").trim()
        } else {
            rest.split(|c: char| c == ',' || c.is_whitespace())
                .next().unwrap_or("").trim()
        };
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
            gems.push(name.to_string());
        }
    }
    gems
}

// ===========================================================================
// Gemfile.lock + gem cache discovery
// ===========================================================================

#[derive(Debug, Clone)]
struct GemEntry {
    name: String,
    version: Option<String>,
}

pub fn parse_gemfile_lock(content: &str) -> Vec<(String, String)> {
    let entries = parse_gemfile_lock_entries(content);
    entries
        .into_iter()
        .filter_map(|e| e.version.map(|v| (e.name, v)))
        .collect()
}

fn parse_gemfile_lock_entries(content: &str) -> Vec<GemEntry> {
    let mut entries = Vec::new();
    let mut in_specs = false;

    for line in content.lines() {
        if line.trim().is_empty() { continue }
        if line == "  specs:" { in_specs = true; continue }
        if !line.starts_with("  ") { in_specs = false; continue }
        if !in_specs { continue }
        if !line.starts_with("    ") || line.starts_with("      ") { continue }

        let trimmed = line.trim();
        if let Some(paren) = trimmed.find(" (") {
            let name = trimmed[..paren].trim().to_string();
            let rest = &trimmed[paren + 2..];
            let version = rest.trim_end_matches(')').trim().to_string();
            if !name.is_empty() && !version.is_empty() {
                entries.push(GemEntry { name, version: Some(version) });
            }
        }
    }
    entries
}

pub fn discover_ruby_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let lock_path = project_root.join("Gemfile.lock");
    let gems: Vec<GemEntry> = if lock_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&lock_path) {
            let parsed = parse_gemfile_lock_entries(&content);
            if !parsed.is_empty() {
                parsed
            } else {
                let gemfile_path = project_root.join("Gemfile");
                if let Ok(gf) = std::fs::read_to_string(&gemfile_path) {
                    parse_gemfile_gems(&gf)
                        .into_iter()
                        .map(|name| GemEntry { name, version: None })
                        .collect()
                } else { return Vec::new() }
            }
        } else { return Vec::new() }
    } else {
        let gemfile_path = project_root.join("Gemfile");
        if !gemfile_path.is_file() { return Vec::new() }
        let Ok(gf) = std::fs::read_to_string(&gemfile_path) else { return Vec::new() };
        parse_gemfile_gems(&gf)
            .into_iter()
            .map(|name| GemEntry { name, version: None })
            .collect()
    };

    if gems.is_empty() { return Vec::new() }

    let candidate_roots = ruby_candidate_gem_roots(project_root);
    if candidate_roots.is_empty() {
        debug!("No bundler gem locations for {}", project_root.display());
        return Vec::new();
    }

    let mut result = Vec::with_capacity(gems.len());
    let mut seen = std::collections::HashSet::new();
    for entry in &gems {
        if !seen.insert(entry.name.clone()) { continue }
        if let Some(gem_root) = find_gem_dir_entry(&candidate_roots, entry) {
            let version = gem_root
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_prefix(&format!("{}-", entry.name)))
                .unwrap_or("")
                .to_string();
            result.push(ExternalDepRoot {
                module_path: entry.name.clone(),
                version,
                root: gem_root,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
            });
        }
    }
    result
}

fn ruby_candidate_gem_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(override_val) = std::env::var("BEARWISDOM_RUBY_GEM_HOME") {
        for seg in std::env::split_paths(&override_val) {
            let gems = seg.join("gems");
            if gems.is_dir() { candidates.push(gems); }
            else if seg.is_dir() { candidates.push(seg); }
        }
    }

    let vendor = project_root.join("vendor").join("bundle").join("ruby");
    if vendor.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&vendor) {
            for entry in entries.flatten() {
                let gems = entry.path().join("gems");
                if gems.is_dir() { candidates.push(gems) }
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        let xdg_gem = home.join(".local").join("share").join("gem").join("ruby");
        if xdg_gem.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&xdg_gem) {
                for entry in entries.flatten() {
                    let gems = entry.path().join("gems");
                    if gems.is_dir() { candidates.push(gems) }
                }
            }
        }
        let gem_dir = home.join(".gem").join("ruby");
        if gem_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&gem_dir) {
                for entry in entries.flatten() {
                    let gems = entry.path().join("gems");
                    if gems.is_dir() { candidates.push(gems) }
                }
            }
        }
        let win_default = home.join("gems").join("gems");
        if win_default.is_dir() { candidates.push(win_default) }
    }

    if let Ok(gem_home) = std::env::var("GEM_HOME") {
        if !gem_home.is_empty() {
            let gems = PathBuf::from(gem_home).join("gems");
            if gems.is_dir() { candidates.push(gems) }
        }
    }
    candidates
}

fn find_gem_dir_entry(candidates: &[PathBuf], entry: &GemEntry) -> Option<PathBuf> {
    let prefix = format!("{}-", entry.name);
    for root in candidates {
        if let Some(ref ver) = entry.version {
            let exact = root.join(format!("{}-{}", entry.name, ver));
            if exact.is_dir() { return Some(exact) }
            let ver_prefix = format!("{}-{}-", entry.name, ver);
            if let Ok(dir_entries) = std::fs::read_dir(root) {
                let mut platform_matches: Vec<PathBuf> = dir_entries
                    .flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        let name = p.file_name()?.to_str()?;
                        if name.starts_with(&ver_prefix) && p.is_dir() { Some(p) } else { None }
                    })
                    .collect();
                if !platform_matches.is_empty() {
                    platform_matches.sort();
                    return platform_matches.pop();
                }
            }
        }

        let Ok(dir_entries) = std::fs::read_dir(root) else { continue };
        let mut matches: Vec<PathBuf> = dir_entries
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                let name = p.file_name()?.to_str()?;
                if name.starts_with(&prefix) && p.is_dir() {
                    let after = &name[prefix.len()..];
                    if after.starts_with(|c: char| c.is_ascii_digit()) { Some(p) } else { None }
                } else { None }
            })
            .collect();
        if !matches.is_empty() {
            matches.sort();
            return matches.pop();
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_ruby_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let lib_dir = dep.root.join("lib");
    if !lib_dir.is_dir() { return Vec::new() }
    let mut out = Vec::new();
    walk_dir_bounded(&lib_dir, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "test" | "tests" | "spec" | "specs" | "bin" | "ext"
                        | "vendor" | "examples" | "docs"
                ) || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".rb") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:ruby:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "ruby",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let r = RubygemsEcosystem;
        assert_eq!(r.id(), ID);
        assert_eq!(Ecosystem::kind(&r), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&r), &["ruby"]);
    }

    #[test]
    fn legacy_locator_tag_is_ruby() {
        assert_eq!(ExternalSourceLocator::ecosystem(&RubygemsEcosystem), "ruby");
    }

    fn capitalize(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }

    fn make_ruby_fixture(tmp: &Path, gems: &[(&str, &str)]) {
        std::fs::create_dir_all(tmp).unwrap();
        let mut gemfile = String::from("source 'https://rubygems.org'\n");
        for (name, _) in gems {
            gemfile.push_str(&format!("gem '{name}'\n"));
        }
        std::fs::write(tmp.join("Gemfile"), gemfile).unwrap();

        let gems_root = tmp.join("vendor").join("bundle").join("ruby").join("3.2.0").join("gems");
        std::fs::create_dir_all(&gems_root).unwrap();
        for (name, version) in gems {
            let gem_root = gems_root.join(format!("{name}-{version}"));
            let lib = gem_root.join("lib");
            std::fs::create_dir_all(&lib).unwrap();
            std::fs::write(
                lib.join(format!("{name}.rb")),
                format!("module {} ; VERSION = '{}' ; end\n", capitalize(name), version),
            ).unwrap();
            std::fs::create_dir_all(gem_root.join("test")).unwrap();
            std::fs::write(gem_root.join("test").join("should_skip.rb"), "# test\n").unwrap();
        }
    }

    #[test]
    fn ruby_locator_finds_vendored_bundle_gems() {
        let tmp = std::env::temp_dir().join("bw-test-rubygems-find");
        let _ = std::fs::remove_dir_all(&tmp);
        make_ruby_fixture(&tmp, &[("devise", "4.9.3"), ("sidekiq", "7.1.0")]);
        let roots = discover_ruby_externals(&tmp);
        assert_eq!(roots.len(), 2);
        let names: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        assert!(names.contains("devise"));
        assert!(names.contains("sidekiq"));
        let devise = roots.iter().find(|r| r.module_path == "devise").unwrap();
        assert_eq!(devise.version, "4.9.3");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ruby_walk_excludes_test_and_spec_dirs() {
        let tmp = std::env::temp_dir().join("bw-test-rubygems-walk");
        let _ = std::fs::remove_dir_all(&tmp);
        make_ruby_fixture(&tmp, &[("devise", "4.9.3")]);
        let roots = discover_ruby_externals(&tmp);
        assert_eq!(roots.len(), 1);
        let walked = walk_ruby_root(&roots[0]);
        assert_eq!(walked.len(), 1);
        let file = &walked[0];
        assert!(file.relative_path.starts_with("ext:ruby:devise/"));
        assert!(file.relative_path.ends_with("lib/devise.rb"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ruby_locator_returns_empty_without_gemfile() {
        let tmp = std::env::temp_dir().join("bw-test-rubygems-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let roots = discover_ruby_externals(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_gemfile_lock_extracts_gem_specs() {
        let lock = r#"GEM
  remote: https://rubygems.org/
  specs:
    actionpack (8.1.3)
      activesupport (= 8.1.3)
    minitest (5.27.0)

PLATFORMS
  x86_64-linux
"#;
        let entries = parse_gemfile_lock_entries(lock);
        assert_eq!(entries.len(), 2);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"actionpack"));
        assert!(names.contains(&"minitest"));
    }

    #[test]
    fn parse_gemfile_lock_handles_platform_variants() {
        let lock = r#"GEM
  specs:
    ffi (1.17.4-x64-mingw-ucrt)
"#;
        let entries = parse_gemfile_lock_entries(lock);
        let ffi = entries.iter().find(|e| e.name == "ffi").unwrap();
        assert_eq!(ffi.version.as_deref(), Some("1.17.4-x64-mingw-ucrt"));
    }

    #[test]
    fn gemfile_lock_used_over_gemfile_when_present() {
        let tmp = std::env::temp_dir().join("bw-test-rubygems-lockfile-preferred");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("Gemfile"), "source 'https://rubygems.org'\ngem 'rails'\n").unwrap();
        let lock = r#"GEM
  specs:
    minitest (5.27.0)
    devise (4.9.3)
"#;
        std::fs::write(tmp.join("Gemfile.lock"), lock).unwrap();

        let gems_root = tmp.join("fake_gems");
        std::fs::create_dir_all(gems_root.join("gems").join("minitest-5.27.0").join("lib")).unwrap();
        std::fs::write(
            gems_root.join("gems").join("minitest-5.27.0").join("lib").join("minitest.rb"),
            "module Minitest; end\n",
        ).unwrap();
        std::fs::create_dir_all(gems_root.join("gems").join("devise-4.9.3").join("lib")).unwrap();
        std::fs::write(
            gems_root.join("gems").join("devise-4.9.3").join("lib").join("devise.rb"),
            "module Devise; end\n",
        ).unwrap();

        std::env::set_var("BEARWISDOM_RUBY_GEM_HOME", gems_root.join("gems").to_str().unwrap());
        let roots = discover_ruby_externals(&tmp);
        std::env::remove_var("BEARWISDOM_RUBY_GEM_HOME");

        let names: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        assert!(names.contains("minitest"));
        assert!(names.contains("devise"));
        assert!(!names.contains("rails"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
