// =============================================================================
// ecosystem/hex.rs — Hex / BEAM ecosystem (Elixir + Erlang + Gleam)
//
// Three languages share the Hex package manager (hex.pm) and the BEAM
// runtime. They differ in install layout:
//
//   - Elixir   — mix places deps under `<project>/deps/<name>/` (project-local)
//   - Erlang   — rebar3 compiles into `<project>/_build/default/lib/<name>/`,
//                OR hex tarballs sit at `~/.hex/packages/hexpm/<name>-<ver>.tar`
//                (shared with Elixir); we extract and cache
//   - Gleam    — gleam fetches into `<project>/build/packages/<name>/`
//
// HexEcosystem runs all three discoveries and unions the roots. The unified
// walker detects source language by extension (.ex/.exs/.erl/.hrl/.gleam)
// so Erlang source inside an Elixir hex dep (e.g. cowboy ships .erl) parses
// correctly.
//
// Before this refactor:
//   indexer/externals/elixir.rs — ElixirExternalsLocator  (~360 LOC)
//   indexer/externals/erlang.rs — ErlangExternalsLocator  (~739 LOC)
//   indexer/externals/gleam.rs  — GleamExternalsLocator   (~93 LOC)
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("hex");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["elixir", "erlang", "gleam"];

// Legacy ecosystem tags written into `ExternalDepRoot::ecosystem` so the
// existing indexer dispatch (keyed on root.ecosystem matching
// locator.ecosystem()) still works. The legacy single-locator-per-ecosystem
// string was "elixir"/"erlang"/"gleam"; we now report "hex" for all three
// since a single HexEcosystem locator handles every walk_root dispatch.
const LEGACY_ECOSYSTEM_TAG: &str = "hex";

pub struct HexEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl (new)
// ---------------------------------------------------------------------------

impl Ecosystem for HexEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("elixir"),
            EcosystemActivation::LanguagePresent("erlang"),
            EcosystemActivation::LanguagePresent("gleam"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_hex_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_hex_root(dep)
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for HexEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_hex_roots(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_hex_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<HexEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(HexEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery — union of three filesystem strategies
// ---------------------------------------------------------------------------

fn discover_hex_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mut roots = Vec::new();
    roots.extend(discover_mix_roots(project_root));
    roots.extend(discover_rebar_roots(project_root));
    roots.extend(discover_gleam_roots(project_root));
    debug!("Hex: {} total external dep roots", roots.len());
    roots
}

// ---------------------------------------------------------------------------
// Elixir (mix) — <project>/deps/<name>/
// ---------------------------------------------------------------------------

fn discover_mix_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mix_exs = project_root.join("mix.exs");
    if !mix_exs.is_file() { return Vec::new() }
    let deps_dir = project_root.join("deps");
    if !deps_dir.is_dir() {
        debug!(
            "No deps/ directory for Elixir project at {} — run `mix deps.get`",
            project_root.display()
        );
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(&deps_dir) else { return Vec::new() };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if !path.join("lib").is_dir() { continue }
        let version = read_mix_package_version(&path).unwrap_or_default();
        out.push(ExternalDepRoot {
            module_path: name.to_string(),
            version,
            root: path,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
        });
    }
    out
}

fn read_mix_package_version(pkg_root: &Path) -> Option<String> {
    let mix_exs = pkg_root.join("mix.exs");
    let content = std::fs::read_to_string(&mix_exs).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("@version ") {
            let rest = rest.trim().trim_start_matches('=').trim();
            let ver = rest.trim_matches('"').trim_matches('\'');
            if !ver.is_empty() { return Some(ver.to_string()) }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Erlang (rebar3) — _build/ OR hex tarball fallback
// ---------------------------------------------------------------------------

fn discover_rebar_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let rebar_config = project_root.join("rebar.config");
    if !rebar_config.is_file() { return Vec::new() }
    let Ok(content) = std::fs::read_to_string(&rebar_config) else { return Vec::new() };
    let declared = parse_rebar_deps(&content);
    if declared.is_empty() { return Vec::new() }

    let locked_versions = parse_rebar_lock(project_root);
    let build_lib = project_root.join("_build").join("default").join("lib");
    let build_available = build_lib.is_dir();
    let hex_cache = hex_packages_dir();

    let mut roots = Vec::new();
    for dep_name in &declared {
        if build_available {
            let dep_dir = build_lib.join(dep_name);
            if dep_dir.is_dir() {
                roots.push(ExternalDepRoot {
                    module_path: dep_name.clone(),
                    version: locked_versions
                        .get(dep_name.as_str())
                        .cloned()
                        .unwrap_or_default(),
                    root: dep_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
                continue;
            }
        }
        if let Some(cache_dir) = hex_cache.as_ref() {
            if let Some((version, extracted)) = locate_hex_dep(
                cache_dir,
                dep_name,
                locked_versions.get(dep_name.as_str()).map(String::as_str),
            ) {
                roots.push(ExternalDepRoot {
                    module_path: dep_name.clone(),
                    version,
                    root: extracted,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
                continue;
            }
        }
        debug!("Erlang: dep '{dep_name}' not found — run `rebar3 compile` to populate");
    }
    roots
}

/// Parse dep names from rebar.config `{deps, [...]}` section.
pub fn parse_rebar_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find("{deps,") else { return deps };
    let rest = &content[start..];
    let Some(bracket_start) = rest.find('[') else { return deps };
    let rest = &rest[bracket_start..];

    let mut bracket_depth = 0i32;
    let mut bracket_end = None;
    for (i, ch) in rest.char_indices() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => {
                bracket_depth -= 1;
                if bracket_depth == 0 { bracket_end = Some(i); break }
            }
            _ => {}
        }
    }
    let bracket_end = match bracket_end { Some(e) => e, None => return deps };
    let deps_block = &rest[1..bracket_end];

    let mut brace_depth = 0u32;
    let mut in_atom = false;
    let mut atom_start = 0usize;
    for (i, ch) in deps_block.char_indices() {
        match ch {
            '{' => {
                brace_depth += 1;
                if brace_depth == 1 { in_atom = true; atom_start = i + 1 }
            }
            ',' | '}' if brace_depth == 1 && in_atom => {
                let name = deps_block[atom_start..i].trim();
                if !name.is_empty()
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    deps.push(name.to_string());
                }
                in_atom = false;
                if ch == '}' { brace_depth = brace_depth.saturating_sub(1) }
            }
            '}' => { brace_depth = brace_depth.saturating_sub(1) }
            _ => {}
        }
    }
    deps
}

pub fn parse_rebar_lock(project_root: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let lock_path = project_root.join("rebar.lock");
    let Ok(content) = std::fs::read_to_string(&lock_path) else { return map };
    let needle = b"{pkg,";
    let bytes = content.as_bytes();
    let mut pos = 0;
    while pos + needle.len() < bytes.len() {
        if bytes[pos..].starts_with(needle) {
            pos += needle.len();
            if let Some((name, after_name)) = read_binary_literal(&content[pos..]) {
                let after_name_pos = pos + after_name;
                if let Some(comma) = content[after_name_pos..].find(',') {
                    let after_comma = after_name_pos + comma + 1;
                    if let Some((version, _)) = read_binary_literal(&content[after_comma..]) {
                        map.insert(name, version);
                    }
                }
            }
        } else {
            pos += 1;
        }
    }
    map
}

fn read_binary_literal(s: &str) -> Option<(String, usize)> {
    let s_trimmed = s.trim_start();
    let rest = s_trimmed.strip_prefix("<<\"")?;
    let end = rest.find("\">>")?;
    let leading = s.len() - s_trimmed.len();
    Some((rest[..end].to_string(), leading + 3 + end + 3))
}

fn hex_packages_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_HEX_PACKAGES") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p) }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".hex").join("packages").join("hexpm");
    if candidate.is_dir() { Some(candidate) } else { None }
}

fn erlang_source_cache_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_ERLANG_SOURCE_CACHE") {
        let p = PathBuf::from(explicit);
        std::fs::create_dir_all(&p).ok()?;
        return Some(p);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(local).join("bearwisdom").join("erlang-sources");
        if std::fs::create_dir_all(&p).is_ok() { return Some(p) }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home).join(".cache").join("bearwisdom").join("erlang-sources");
        if std::fs::create_dir_all(&p).is_ok() { return Some(p) }
    }
    None
}

fn locate_hex_dep(
    hex_cache: &Path,
    dep_name: &str,
    pinned_version: Option<&str>,
) -> Option<(String, PathBuf)> {
    let (tar_path, version) = if let Some(ver) = pinned_version {
        let p = hex_cache.join(format!("{dep_name}-{ver}.tar"));
        if p.is_file() { (p, ver.to_string()) } else { return None }
    } else {
        let entries = std::fs::read_dir(hex_cache).ok()?;
        let prefix = format!("{dep_name}-");
        let mut candidates: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let stripped = name.strip_prefix(&prefix)?.strip_suffix(".tar")?;
                let path = e.path();
                if path.is_file() { Some((stripped.to_string(), path)) } else { None }
            })
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        let (ver, path) = candidates.into_iter().next_back()?;
        (path, ver)
    };

    let cache_base = erlang_source_cache_dir()?;
    let extracted = cache_base.join(format!("{dep_name}-{version}"));

    if extracted.is_dir() && !is_hex_cache_stale(&tar_path, &extracted) {
        return Some((version, extracted));
    }

    match extract_hex_tarball(&tar_path, &extracted) {
        Ok(()) => Some((version, extracted)),
        Err(e) => {
            debug!("Erlang hex: failed to extract {dep_name}-{version}: {e}");
            None
        }
    }
}

fn is_hex_cache_stale(tar: &Path, cache_dir: &Path) -> bool {
    let tar_mtime = match std::fs::metadata(tar).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let Ok(entries) = std::fs::read_dir(cache_dir) else { return true };
    let mut newest: Option<std::time::SystemTime> = None;
    for entry in entries.flatten() {
        if let Ok(md) = entry.metadata() {
            if let Ok(t) = md.modified() {
                newest = Some(newest.map(|cur| cur.max(t)).unwrap_or(t));
            }
        }
    }
    match newest { Some(t) => tar_mtime > t, None => true }
}

fn extract_hex_tarball(tar_path: &Path, dest: &Path) -> std::io::Result<()> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    std::fs::create_dir_all(dest)?;
    let outer_file = std::fs::File::open(tar_path)?;
    let mut outer = ::tar::Archive::new(outer_file);

    for entry in outer.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let name = path.to_string_lossy();
        if name != "contents.tar.gz" { continue }
        let mut gz_bytes = Vec::new();
        entry.read_to_end(&mut gz_bytes)?;
        let gz_cursor = std::io::Cursor::new(gz_bytes);
        let gz_decoder = GzDecoder::new(gz_cursor);
        let mut inner = ::tar::Archive::new(gz_decoder);

        for inner_entry in inner.entries()? {
            let mut inner_entry = inner_entry?;
            let inner_path = inner_entry.path()?.to_path_buf();
            let Some(file_name) = inner_path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(file_name.ends_with(".erl") || file_name.ends_with(".hrl")) { continue }
            if file_name.ends_with("_SUITE.erl") || file_name.ends_with("_tests.erl") { continue }
            let out_path = dest.join(&inner_path);
            let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf());
            let canonical_out = match out_path.parent() {
                Some(parent) => {
                    if std::fs::create_dir_all(parent).is_err() { continue }
                    parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf())
                }
                None => continue,
            };
            if !canonical_out.starts_with(&canonical_dest) { continue }
            let mut out_file = std::fs::File::create(&out_path)?;
            std::io::copy(&mut inner_entry, &mut out_file)?;
        }
        return Ok(());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "contents.tar.gz not found in hex tarball",
    ))
}

// ---------------------------------------------------------------------------
// Gleam — <project>/build/packages/<name>/
// ---------------------------------------------------------------------------

fn discover_gleam_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::ecosystem::manifest::gleam::parse_gleam_deps;

    let gleam_toml = project_root.join("gleam.toml");
    if !gleam_toml.is_file() { return Vec::new() }
    let Ok(content) = std::fs::read_to_string(&gleam_toml) else { return Vec::new() };
    let declared = parse_gleam_deps(&content);
    if declared.is_empty() { return Vec::new() }

    let packages = project_root.join("build").join("packages");
    if !packages.is_dir() { return Vec::new() }

    let mut out = Vec::new();
    for dep in &declared {
        let dep_dir = packages.join(dep);
        if dep_dir.is_dir() {
            out.push(ExternalDepRoot {
                module_path: dep.clone(),
                version: String::new(),
                root: dep_dir,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Walk: single recursive walker. Starts at dep.root; skips non-source
// directories; emits .ex/.exs/.erl/.hrl/.gleam with per-file language.
// ---------------------------------------------------------------------------

fn walk_hex_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    // Start at conventional source subdirs when they exist; this matches
    // the behavior of the original per-language walkers:
    //   Elixir: lib/
    //   Erlang: src/ + include/
    //   Gleam:  src/ (fallback to root)
    // Walking the package root directly would pick up mix.exs,
    // rebar.config, and other build scripts that the per-language walkers
    // intentionally excluded.
    let mut any_subdir = false;
    for subdir in &["lib", "src", "include"] {
        let d = dep.root.join(subdir);
        if d.is_dir() {
            walk_dir_bounded(&d, &dep.root, dep, &mut out, 0);
            any_subdir = true;
        }
    }
    // Gleam packages may ship flat (no src/). Fall back to walking the root
    // when no conventional source subdir exists.
    if !any_subdir {
        walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
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
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "test" | "tests" | "priv" | "bin" | "config"
                        | "doc" | "docs" | "assets" | "examples" | "_build"
                        | "cover" | "ebin" | "deps" | "target"
                ) || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let (language, virtual_tag) = match detect_hex_language(name) {
                Some(spec) => spec,
                None => continue,
            };
            // Skip test-suffixed files.
            if name.ends_with("_SUITE.erl") || name.ends_with("_tests.erl") { continue }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:{virtual_tag}:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

fn detect_hex_language(name: &str) -> Option<(&'static str, &'static str)> {
    if name.ends_with(".ex") || name.ends_with(".exs") {
        Some(("elixir", "elixir"))
    } else if name.ends_with(".erl") || name.ends_with(".hrl") {
        Some(("erlang", "erlang"))
    } else if name.ends_with(".gleam") {
        Some(("gleam", "gleam"))
    } else {
        None
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
        let h = HexEcosystem;
        assert_eq!(h.id(), ID);
        assert_eq!(Ecosystem::kind(&h), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&h), &["elixir", "erlang", "gleam"]);
    }

    #[test]
    fn legacy_locator_tag_is_hex() {
        assert_eq!(ExternalSourceLocator::ecosystem(&HexEcosystem), "hex");
    }

    #[test]
    fn detect_hex_language_covers_extensions() {
        assert_eq!(detect_hex_language("foo.ex"), Some(("elixir", "elixir")));
        assert_eq!(detect_hex_language("foo.exs"), Some(("elixir", "elixir")));
        assert_eq!(detect_hex_language("bar.erl"), Some(("erlang", "erlang")));
        assert_eq!(detect_hex_language("bar.hrl"), Some(("erlang", "erlang")));
        assert_eq!(detect_hex_language("baz.gleam"), Some(("gleam", "gleam")));
        assert_eq!(detect_hex_language("readme.md"), None);
    }

    // --- Elixir/mix tests ---

    fn capitalize(s: &str) -> String {
        let mut c = s.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }

    fn make_elixir_fixture(tmp: &Path, deps: &[&str]) {
        std::fs::create_dir_all(tmp).unwrap();
        let mut mix = String::from(
            "defmodule MyApp.MixProject do\n  use Mix.Project\n  defp deps do\n    [\n",
        );
        for name in deps {
            mix.push_str(&format!("      {{:{name}, \"~> 1.0\"}},\n"));
        }
        mix.push_str("    ]\n  end\nend\n");
        std::fs::write(tmp.join("mix.exs"), mix).unwrap();

        for name in deps {
            let pkg = tmp.join("deps").join(name);
            let lib = pkg.join("lib");
            std::fs::create_dir_all(&lib).unwrap();
            std::fs::write(
                lib.join(format!("{name}.ex")),
                format!(
                    "defmodule {} do\n  def hello, do: :world\nend\n",
                    capitalize(name)
                ),
            )
            .unwrap();
            std::fs::write(
                pkg.join("mix.exs"),
                format!(
                    "defmodule {}.MixProject do\n  @version \"1.2.3\"\nend\n",
                    capitalize(name)
                ),
            )
            .unwrap();
            std::fs::create_dir_all(pkg.join("test")).unwrap();
            std::fs::write(pkg.join("test").join("should_skip.exs"), "# test\n").unwrap();
            std::fs::create_dir_all(pkg.join("priv")).unwrap();
            std::fs::write(pkg.join("priv").join("seeds.exs"), "# priv\n").unwrap();
        }
    }

    #[test]
    fn mix_locator_finds_deps_directories() {
        let tmp = std::env::temp_dir().join("bw-test-hex-mix-find");
        let _ = std::fs::remove_dir_all(&tmp);
        make_elixir_fixture(&tmp, &["phoenix", "ecto", "plug"]);

        let roots = discover_mix_roots(&tmp);
        assert_eq!(roots.len(), 3);
        let names: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        assert!(names.contains("phoenix"));
        assert!(names.contains("ecto"));
        assert!(names.contains("plug"));
        assert!(roots.iter().all(|r| r.version == "1.2.3"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn mix_walk_excludes_test_priv_and_config() {
        let tmp = std::env::temp_dir().join("bw-test-hex-mix-walk");
        let _ = std::fs::remove_dir_all(&tmp);
        make_elixir_fixture(&tmp, &["phoenix"]);

        let roots = discover_mix_roots(&tmp);
        assert_eq!(roots.len(), 1);
        let walked = walk_hex_root(&roots[0]);
        assert_eq!(walked.len(), 1);
        let file = &walked[0];
        assert!(file.relative_path.starts_with("ext:elixir:phoenix/"));
        assert!(file.relative_path.ends_with("lib/phoenix.ex"));
        assert_eq!(file.language, "elixir");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn mix_returns_empty_without_mix_exs() {
        let tmp = std::env::temp_dir().join("bw-test-hex-mix-no-manifest");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let roots = discover_mix_roots(&tmp);
        assert!(roots.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // --- rebar (Erlang) tests ---

    #[test]
    fn erlang_parses_rebar_deps_git() {
        let content = r#"{deps, [
{cowlib,".*",{git,"https://github.com/ninenines/cowlib",{tag,"2.16.0"}}},
{ranch,".*",{git,"https://github.com/ninenines/ranch",{tag,"1.8.1"}}}
]}."#;
        let deps = parse_rebar_deps(content);
        assert_eq!(deps, vec!["cowlib", "ranch"]);
    }

    #[test]
    fn erlang_parses_rebar_deps_hex_shorthand() {
        let content = r#"{deps, [{cowlib, "~> 2.12"}, {ranch, "~> 1.8"}]}."#;
        let deps = parse_rebar_deps(content);
        assert_eq!(deps, vec!["cowlib", "ranch"]);
    }

    #[test]
    fn erlang_parses_rebar_lock_versions() {
        let tmp = std::env::temp_dir().join("bw-test-hex-rebar-lock");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("rebar.lock"),
            r#"{"1.2.0",
[{<<"cowlib">>,{pkg,<<"cowlib">>,<<"2.16.0">>,<<"HASH1">>,<<"HASH2">>},0},
 {<<"ranch">>,{pkg,<<"ranch">>,<<"1.8.1">>,<<"HASH3">>,<<"HASH4">>},0}]}."#,
        )
        .unwrap();
        let versions = parse_rebar_lock(&tmp);
        assert_eq!(versions.get("cowlib").map(String::as_str), Some("2.16.0"));
        assert_eq!(versions.get("ranch").map(String::as_str), Some("1.8.1"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn erlang_discovers_build_deps() {
        let tmp = std::env::temp_dir().join("bw-test-hex-rebar-build");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("rebar.config"),
            r#"{deps, [{cowlib,".*",{git,"url",{tag,"1.0"}}},{ranch,".*",{git,"url",{tag,"1.0"}}}]}."#,
        )
        .unwrap();
        let deps_dir = tmp.join("_build").join("default").join("lib");
        let cowlib_src = deps_dir.join("cowlib").join("src");
        std::fs::create_dir_all(&cowlib_src).unwrap();
        std::fs::write(cowlib_src.join("cowlib.erl"), "-module(cowlib).\n").unwrap();

        let empty_hex = tmp.join("empty-hex");
        std::fs::create_dir_all(&empty_hex).unwrap();
        std::env::set_var("BEARWISDOM_HEX_PACKAGES", &empty_hex);

        let roots = discover_rebar_roots(&tmp);
        std::env::remove_var("BEARWISDOM_HEX_PACKAGES");

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].module_path, "cowlib");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn walk_skips_suite_and_test_files() {
        let tmp = std::env::temp_dir().join("bw-test-hex-walk-skip");
        let _ = std::fs::remove_dir_all(&tmp);
        let pkg_root = tmp.join("cowlib");
        let src = pkg_root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("cowlib.erl"), "-module(cowlib).\n").unwrap();
        std::fs::write(src.join("cowlib_SUITE.erl"), "% test suite\n").unwrap();
        std::fs::write(src.join("cowlib_tests.erl"), "% unit tests\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "cowlib".into(),
            version: "2.16.0".into(),
            root: pkg_root,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
        };
        let walked = walk_hex_root(&dep);
        assert_eq!(walked.len(), 1);
        assert!(walked[0].relative_path.ends_with("src/cowlib.erl"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
