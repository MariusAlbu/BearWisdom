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

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
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

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_ruby_gem_entry(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        resolve_ruby_gem_entry(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_ruby_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
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
                requested_imports: Vec::new(),
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
// Reachability: gem entry + bounded require expansion
// ---------------------------------------------------------------------------

const RB_REQUIRE_MAX_DEPTH: u32 = 3;

/// Start from the gem's primary entry file (`lib/<gem-name>.rb`, with a
/// fallback to `lib/<dash-to-slash>.rb` for namespaced gems like
/// `rails-html-sanitizer` → `lib/rails/html/sanitizer.rb`) and follow
/// `require "<gem>/..."` + `require_relative "..."` calls bounded at
/// depth 3. Cross-gem requires are skipped — they're separate dep roots.
fn resolve_ruby_gem_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let lib = dep.root.join("lib");
    if !lib.is_dir() { return Vec::new() }

    let primary = lib.join(format!("{}.rb", dep.module_path));
    let dashed = lib.join(format!("{}.rb", dep.module_path.replace('-', "/")));
    let entry = if primary.is_file() {
        primary
    } else if dashed.is_file() {
        dashed
    } else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    expand_ruby_requires_into(dep, &lib, &entry, &mut out, &mut seen, 0);
    out
}

fn expand_ruby_requires_into(
    dep: &ExternalDepRoot,
    lib_root: &Path,
    file: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !seen.insert(file.to_path_buf()) { return }
    if !file.is_file() { return }

    let rel_sub = match file.strip_prefix(&dep.root) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => return,
    };
    out.push(WalkedFile {
        relative_path: format!("ext:ruby:{}/{}", dep.module_path, rel_sub),
        absolute_path: file.to_path_buf(),
        language: "ruby",
    });

    if depth >= RB_REQUIRE_MAX_DEPTH { return }

    let Ok(src) = std::fs::read_to_string(file) else { return };
    for (kind, target) in extract_ruby_requires(&src) {
        let next = match kind {
            // Ruby's $LOAD_PATH prepends every gem's lib/, so
            // `require "devise/version"` resolves to `lib/devise/version.rb`
            // in whichever gem ships it. Probe this gem's lib first; if the
            // file isn't here, treat it as a separate dep root (skip).
            RubyRequireKind::Absolute => resolve_ruby_lib_path(lib_root, &target),
            RubyRequireKind::Relative => resolve_ruby_relative_path(file, &target),
        };
        let Some(next) = next else { continue };
        expand_ruby_requires_into(dep, lib_root, &next, out, seen, depth + 1);
    }
}

enum RubyRequireKind { Absolute, Relative }

/// Scan line-oriented for `require "..."`, `require '...'`,
/// `require_relative "..."`, `require_relative '...'`. Returns
/// (kind, spec) for each match. Ignores string interpolation and
/// dynamic requires (heredocs, variables).
fn extract_ruby_requires(src: &str) -> Vec<(RubyRequireKind, String)> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let line = raw.trim_start();
        // Strip inline comments (Ruby's `#` but not inside strings).
        // Line-oriented, conservative — stop at first `#` that's not at a word-char boundary.
        let (kind, rest) = if let Some(r) = line.strip_prefix("require_relative ") {
            (RubyRequireKind::Relative, r)
        } else if let Some(r) = line.strip_prefix("require ") {
            (RubyRequireKind::Absolute, r)
        } else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(q) = rest.chars().next() else { continue };
        if q != '\'' && q != '"' { continue }
        let inner = &rest[1..];
        let Some(end) = inner.find(q) else { continue };
        let spec = &inner[..end];
        if spec.is_empty() { continue }
        out.push((kind, spec.to_string()));
    }
    out
}

/// Resolve a `require "foo/bar"` spec to `<lib_root>/foo/bar.rb` within
/// this gem. Returns None if the file isn't under this gem's lib — that
/// means the require targets a different gem and will be handled by its
/// own dep root.
fn resolve_ruby_lib_path(lib_root: &Path, spec: &str) -> Option<PathBuf> {
    if spec.is_empty() { return None }
    let candidate = lib_root.join(format!("{spec}.rb"));
    if candidate.is_file() { Some(candidate) } else { None }
}

/// Resolve `require_relative "X"` against the file's directory. Spec is
/// a path without the `.rb` suffix; resolves to `<dir>/<X>.rb`.
fn resolve_ruby_relative_path(from_file: &Path, spec: &str) -> Option<PathBuf> {
    let base = from_file.parent()?;
    let candidate = base.join(format!("{spec}.rb"));
    if candidate.is_file() { Some(candidate) } else { None }
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
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

fn build_ruby_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_ruby_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_ruby_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();
    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Header-only tree-sitter scan of a Ruby source file. Records top-level
/// class / module / method names. Nested classes are surfaced as bare names
/// so `find_by_name` can resolve `ActiveRecord::Base` → the file defining
/// `Base`. Function bodies are not walked but Ruby's body-less declaration
/// model means we still descend into class / module bodies to capture inner
/// classes and def'd methods.
fn scan_ruby_header(source: &str) -> Vec<String> {
    let language = tree_sitter_ruby::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    walk_ruby_decls(&root, bytes, &mut out, 0);
    out
}

fn walk_ruby_decls(node: &Node, bytes: &[u8], out: &mut Vec<String>, depth: u32) {
    if depth > 4 { return }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "class" | "module" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(t) = name_node.utf8_text(bytes) {
                        // Name can be `Base` or `Foo::Bar` — record as-is and
                        // also the last segment so both lookups hit.
                        out.push(t.to_string());
                        if let Some(last) = t.rsplit("::").next() {
                            if last != t { out.push(last.to_string()) }
                        }
                    }
                }
                // Descend into body to capture nested class / module / def.
                walk_ruby_decls(&child, bytes, out, depth + 1);
            }
            "method" | "singleton_method" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(t) = name_node.utf8_text(bytes) {
                        out.push(t.to_string());
                    }
                }
            }
            "body_statement" | "program" | "begin_block" => {
                walk_ruby_decls(&child, bytes, out, depth + 1);
            }
            _ => {}
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

    // -----------------------------------------------------------------
    // R3 — reachability-based entry resolution
    // -----------------------------------------------------------------

    fn mkdep(root: PathBuf, name: &str) -> ExternalDepRoot {
        ExternalDepRoot {
            module_path: name.to_string(),
            version: String::new(),
            root,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        }
    }

    #[test]
    fn extract_ruby_requires_finds_both_forms() {
        let src = r#"
require "active_support/core_ext"
require_relative "helpers"
require 'devise/rails'
require_relative 'config/strategies'
require foo # dynamic — skipped
        "#;
        let reqs = extract_ruby_requires(src);
        assert_eq!(reqs.len(), 4);
        let specs: Vec<String> = reqs.iter().map(|(_, s)| s.clone()).collect();
        assert!(specs.contains(&"active_support/core_ext".to_string()));
        assert!(specs.contains(&"helpers".to_string()));
        assert!(specs.contains(&"devise/rails".to_string()));
        assert!(specs.contains(&"config/strategies".to_string()));
    }

    #[test]
    fn resolve_entry_follows_in_gem_requires() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("devise-4.9.3");
        let lib = root.join("lib");
        std::fs::create_dir_all(lib.join("devise")).unwrap();
        std::fs::write(
            lib.join("devise.rb"),
            r#"
require "devise/version"
require_relative "devise/helpers"
require "otheram/stuff"
"#,
        ).unwrap();
        std::fs::write(lib.join("devise").join("version.rb"), "module Devise; VERSION='x'; end\n").unwrap();
        std::fs::write(lib.join("devise").join("helpers.rb"), "module Devise::Helpers; end\n").unwrap();

        let dep = mkdep(root.clone(), "devise");
        let files = RubygemsEcosystem.resolve_import(&dep, "devise", &[]);
        assert_eq!(files.len(), 3, "got: {:?}", files);
        let paths: std::collections::HashSet<_> =
            files.iter().map(|f| f.absolute_path.clone()).collect();
        assert!(paths.contains(&lib.join("devise.rb")));
        assert!(paths.contains(&lib.join("devise").join("version.rb")));
        assert!(paths.contains(&lib.join("devise").join("helpers.rb")));
        for f in &files {
            assert!(f.relative_path.starts_with("ext:ruby:devise/"));
            assert_eq!(f.language, "ruby");
        }
    }

    #[test]
    fn resolve_entry_falls_back_to_dashed_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("rails-html-sanitizer-1.0.0");
        let nested = root.join("lib").join("rails").join("html");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("sanitizer.rb"), "module Rails::Html::Sanitizer; end\n").unwrap();

        let dep = mkdep(root, "rails-html-sanitizer");
        let files = RubygemsEcosystem.resolve_import(&dep, "rails-html-sanitizer", &[]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].absolute_path, nested.join("sanitizer.rb"));
    }

    #[test]
    fn resolve_entry_empty_when_no_lib() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("missing-0.1.0");
        std::fs::create_dir_all(&root).unwrap();

        let dep = mkdep(root, "missing");
        assert!(RubygemsEcosystem.resolve_import(&dep, "missing", &[]).is_empty());
    }
}
