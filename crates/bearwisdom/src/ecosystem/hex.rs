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

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
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

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        // Hex covers the Erlang/Elixir/Gleam triumvirate. Each tool brings
        // its own manifest filename; map them to distinct kinds so users
        // can tell them apart in queries.
        &[
            ("mix.exs",      "elixir"),
            ("rebar.config", "erlang"),
            ("gleam.toml",   "gleam"),
        ]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["_build", "deps"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via mix.exs / gleam.toml. A bare directory of
        // `.ex`/`.erl`/`.gleam` files without a manifest can't be
        // resolved against external Hex coordinates, so dropping the
        // LanguagePresent shotgun is correct per the trait doc.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_hex_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_hex_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        walk_hex_narrowed(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        _fqn: &str,
    ) -> Vec<WalkedFile> {
        walk_hex_narrowed(dep)
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_hex_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
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
    // R3: scan project source once, attach the demand set to every dep root.
    // Each language's narrowing logic interprets these as its own conventions
    // (Elixir/Gleam → file path, Erlang → module-name match).
    let user_imports: Vec<String> = collect_hex_user_imports(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();
    roots.extend(discover_mix_roots(project_root, &user_imports));
    roots.extend(discover_rebar_roots(project_root, &user_imports));
    roots.extend(discover_erlang_mk_roots(project_root, &user_imports));
    roots.extend(discover_gleam_roots(project_root, &user_imports));
    debug!("Hex: {} total external dep roots", roots.len());
    roots
}

// ---------------------------------------------------------------------------
// Erlang (erlang.mk) — <project>/deps/<name>/, populated by `make`
// ---------------------------------------------------------------------------
//
// erlang.mk uses Makefile variable expansion (`DEPS = $(PLUGINS)`) that we
// can't evaluate without invoking make. Instead we trust the populated
// deps/ directory: every subdir is treated as an external dep root. That's
// also what `rebar3 compile` produces under `_build/default/lib/`, so the
// downstream walker code reuses the same path.
//
// Activation gate: `erlang.mk` file at project root. Without it we don't
// fire, even on a project that happens to have a `deps/` directory (could
// be Elixir's mix layout, which has its own discovery).

fn discover_erlang_mk_roots(
    project_root: &Path,
    user_imports: &[String],
) -> Vec<ExternalDepRoot> {
    if !project_root.join("erlang.mk").is_file() {
        return Vec::new();
    }
    let deps_dir = project_root.join("deps");
    if !deps_dir.is_dir() {
        debug!(
            "erlang.mk project at {} has no deps/ — run `make deps`",
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
        if name.starts_with('.') { continue }
        out.push(ExternalDepRoot {
            module_path: name.to_string(),
            // erlang.mk doesn't pin versions in Makefile DEPS; the version
            // is encoded in the dep's own .app.src or .app file. Leave
            // empty for MVP — module_path is what the resolver matches on.
            version: String::new(),
            root: path,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: user_imports.to_vec(),
        });
    }
    debug!("erlang.mk: {} dep roots from {}", out.len(), deps_dir.display());
    out
}

// ---------------------------------------------------------------------------
// Elixir (mix) — <project>/deps/<name>/
// ---------------------------------------------------------------------------

fn discover_mix_roots(project_root: &Path, user_imports: &[String]) -> Vec<ExternalDepRoot> {
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
            requested_imports: user_imports.to_vec(),
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

fn discover_rebar_roots(project_root: &Path, user_imports: &[String]) -> Vec<ExternalDepRoot> {
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
                    requested_imports: user_imports.to_vec(),
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
                    requested_imports: user_imports.to_vec(),
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

fn discover_gleam_roots(project_root: &Path, user_imports: &[String]) -> Vec<ExternalDepRoot> {
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
                requested_imports: user_imports.to_vec(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// R3 reachability — scan project for module references, narrow walk
// ---------------------------------------------------------------------------
//
// Each language uses a different convention but they all map module names
// onto file paths:
//   - Elixir: `alias Foo.Bar` / `import Foo.Bar` / `use Foo.Bar` / `Foo.Bar.fn()`
//             → file `foo/bar.ex` under `lib/`
//   - Erlang: `foo:bar()` / `-include("foo.hrl").` → `foo.erl` / `foo.hrl`
//   - Gleam:  `import foo/bar` → `foo/bar.gleam` under `src/`
//
// We collect every module reference once across the project and store the
// raw set on each ExternalDepRoot. walk_hex_narrowed maps each reference to
// candidate path tails and keeps only files matching them, plus same-dir
// siblings (same-module-namespace types/functions don't get a fresh
// reference but still need walking).

fn collect_hex_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_hex_imports_recursive(project_root, &mut out, 0);
    out
}

fn scan_hex_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    ".git" | "deps" | "_build" | "node_modules" | "build"
                        | "priv" | "ebin" | "cover" | "doc" | "docs"
                        | "assets" | "tmp" | "target"
                ) || name.starts_with('.') { continue }
            }
            scan_hex_imports_recursive(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            if name.ends_with(".ex") || name.ends_with(".exs") {
                extract_elixir_module_refs(&content, out);
            } else if name.ends_with(".erl") || name.ends_with(".hrl") {
                extract_erlang_module_refs(&content, out);
            } else if name.ends_with(".gleam") {
                extract_gleam_module_refs(&content, out);
            }
        }
    }
}

/// Capture `alias Foo.Bar` / `alias Foo.{Bar, Baz}` / `import Foo` / `use Foo` /
/// `Foo.Bar.fun()` / `%Foo.Bar{}`. Stored as Elixir module names (dotted) — the
/// narrowing pass converts each to a `lib/foo/bar.ex` tail.
fn extract_elixir_module_refs(content: &str, out: &mut std::collections::HashSet<String>) {
    for raw in content.lines() {
        let line = raw.trim();
        // `alias Foo.{Bar, Baz}`
        if let Some(rest) = line.strip_prefix("alias ") {
            collect_elixir_dotted_or_braced(rest, out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("import ") {
            collect_elixir_dotted_or_braced(rest, out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("use ") {
            collect_elixir_dotted_or_braced(rest, out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ") {
            collect_elixir_dotted_or_braced(rest, out);
            continue;
        }
        // Inline references (`Foo.Bar.func`, `%Foo.Bar{}`). Walk the line for
        // capitalised dotted runs. Conservative — false positives just walk
        // an extra file, which is the failure mode we tolerate.
        scan_elixir_module_tokens(line, out);
    }
}

fn collect_elixir_dotted_or_braced(rest: &str, out: &mut std::collections::HashSet<String>) {
    let rest = rest.trim();
    // Brace block first (before any `,` split, since the block itself contains commas).
    if let Some(brace_open) = rest.find('{') {
        if let Some(brace_close) = rest.find('}') {
            let prefix = rest[..brace_open].trim_end_matches('.').trim();
            if prefix.is_empty() { return }
            let inner = &rest[brace_open + 1..brace_close];
            for sel in inner.split(',') {
                let sel = sel.trim();
                if sel.is_empty() { continue }
                out.insert(format!("{prefix}.{sel}"));
            }
            return;
        }
    }
    // Single dotted name: stop at the first `,`/whitespace/options keyword.
    let head = rest
        .split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim_end_matches(',');
    if !head.is_empty() && head.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
        out.insert(head.to_string());
    }
}

fn scan_elixir_module_tokens(line: &str, out: &mut std::collections::HashSet<String>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            let tok = &line[start..i];
            if tok.contains('.')
                && tok.split('.').all(|seg| {
                    !seg.is_empty()
                        && seg.chars().next().map_or(false, |c| c.is_ascii_uppercase())
                })
            {
                out.insert(tok.to_string());
            }
        } else {
            i += 1;
        }
    }
}

/// Erlang module references appear as `foo:bar(...)` calls and
/// `-include("foo.hrl").` directives. Stored as bare module/header names.
fn extract_erlang_module_refs(content: &str, out: &mut std::collections::HashSet<String>) {
    for raw in content.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("-include(\"") {
            if let Some(end) = rest.find('"') {
                let header = &rest[..end];
                if !header.is_empty() { out.insert(header.to_string()); }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("-include_lib(\"") {
            if let Some(end) = rest.find('"') {
                let header = &rest[..end];
                if let Some(slash) = header.rfind('/') {
                    out.insert(header[slash + 1..].to_string());
                } else {
                    out.insert(header.to_string());
                }
            }
            continue;
        }
        // `foo:bar(...)` — only first-segment matters; header tokens get the
        // bare module name (`foo`).
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b.is_ascii_lowercase() {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b':' {
                    let module = &line[start..i];
                    if !module.is_empty() {
                        out.insert(format!("{module}.erl"));
                    }
                }
            } else {
                i += 1;
            }
        }
    }
}

/// Gleam imports: `import foo/bar` → store as `foo/bar` (path-shaped).
fn extract_gleam_module_refs(content: &str, out: &mut std::collections::HashSet<String>) {
    for raw in content.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("import ") else { continue };
        let head = rest
            .split_whitespace()
            .next()
            .unwrap_or("");
        let head = head.split('.').next().unwrap_or("");
        if head.is_empty() { continue }
        out.insert(format!("gleam:{head}"));
    }
}

/// Build the set of file path tails the narrow walk should match. We expand
/// each requested ref into language-specific candidate tails so a single
/// walked file can satisfy multiple convention checks.
fn requested_to_path_suffixes(
    refs: &[String],
) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for r in refs {
        // Gleam-tagged refs: `gleam:foo/bar` → `foo/bar.gleam`
        if let Some(path) = r.strip_prefix("gleam:") {
            out.insert(format!("{}.gleam", path.replace('.', "/")));
            continue;
        }
        // Erlang refs already carry an extension.
        if r.ends_with(".erl") || r.ends_with(".hrl") {
            out.insert(r.clone());
            continue;
        }
        // Elixir module: `Foo.Bar.Baz` → `lib/foo/bar/baz.ex`. We emit two
        // tails: the snake_cased file path AND each parent path so deep
        // modules still match when only a leaf file holds the dep.
        let snake = r
            .split('.')
            .map(elixir_to_snake)
            .collect::<Vec<_>>()
            .join("/");
        if !snake.is_empty() {
            out.insert(format!("{snake}.ex"));
            out.insert(format!("{snake}.exs"));
        }
    }
    out
}

/// `FooBarBaz` → `foo_bar_baz`. Elixir module-to-filename convention.
fn elixir_to_snake(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len() + 4);
    for (i, ch) in seg.char_indices() {
        if ch.is_ascii_uppercase() {
            if i > 0 { out.push('_') }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn walk_hex_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() {
        return walk_hex_root(dep);
    }
    let suffixes = requested_to_path_suffixes(&dep.requested_imports);
    if suffixes.is_empty() {
        return walk_hex_root(dep);
    }

    let mut out = Vec::new();
    let mut any_subdir = false;
    for subdir in &["lib", "src", "include"] {
        let d = dep.root.join(subdir);
        if d.is_dir() {
            walk_narrowed_dir(&d, &dep.root, dep, &suffixes, &mut out, 0);
            any_subdir = true;
        }
    }
    if !any_subdir {
        walk_narrowed_dir(&dep.root, &dep.root, dep, &suffixes, &mut out, 0);
    }
    out
}

fn walk_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    suffixes: &std::collections::HashSet<String>,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut dir_files: Vec<(PathBuf, String, &'static str, &'static str)> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut any_match = false;

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
            subdirs.push(path);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let Some((language, virtual_tag)) = detect_hex_language(name) else { continue };
            if name.ends_with("_SUITE.erl") || name.ends_with("_tests.erl") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if suffixes.iter().any(|s| rel_sub.ends_with(s)) {
                any_match = true;
            }
            dir_files.push((path, rel_sub, language, virtual_tag));
        }
    }

    if any_match {
        for (path, rel_sub, language, virtual_tag) in dir_files {
            let virtual_path = format!("ext:{virtual_tag}:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }

    for sub in subdirs {
        walk_narrowed_dir(&sub, root, dep, suffixes, out, depth + 1);
    }
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
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

pub(crate) fn build_hex_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_hex_root(dep) {
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
            let names = match wf.language {
                "elixir" => scan_elixir_header(&src),
                "erlang" => scan_erlang_header(&src),
                "gleam" => scan_gleam_header(&src),
                _ => Vec::new(),
            };
            names
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

/// Header-only tree-sitter scan of an Elixir source file. Records every
/// top-level `defmodule Foo.Bar` name and, inside each module body, the
/// names declared via `def`, `defp`, `defmacro`, `defstruct`, `defguard`,
/// `defmodule` (nested), `def type`, `defprotocol`, `defimpl`. Function
/// bodies are never descended.
fn scan_elixir_header(source: &str) -> Vec<String> {
    let language = tree_sitter_elixir::LANGUAGE.into();
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
    walk_elixir_body(&root, bytes, &mut out, 0);
    out
}

fn walk_elixir_body(node: &Node, bytes: &[u8], out: &mut Vec<String>, depth: u32) {
    if depth > 6 { return }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Elixir's grammar represents every form as a `call` node whose first
        // `identifier` child is the macro name (`defmodule`, `def`, `defp`,
        // `defmacro`, `defstruct`, `defprotocol`, `defimpl`, `defguard`),
        // followed by the form's arguments. We walk the whole tree because
        // module-body statements are wrapped in `do_block` / `body` nodes
        // under the outer call.
        if child.kind() == "call" {
            let mut ic = child.walk();
            let children: Vec<Node> = child.children(&mut ic).collect();
            if let Some(head) = children.iter().find(|n| n.kind() == "identifier") {
                if let Ok(head_text) = head.utf8_text(bytes) {
                    let is_decl = matches!(
                        head_text,
                        "defmodule" | "def" | "defp" | "defmacro" | "defmacrop"
                            | "defstruct" | "defprotocol" | "defimpl" | "defguard"
                            | "defguardp" | "defdelegate" | "defexception" | "defcallback"
                    );
                    if is_decl {
                        if let Some(args) = children.iter().find(|n| n.kind() == "arguments") {
                            if let Some(name) = first_elixir_arg_name(args, bytes) {
                                out.push(name);
                            }
                        }
                    }
                }
            }
        }
        // Always recurse so nested def/defmodule inside do_block body get
        // visited — the grammar introduces `do_block` / `stab_clause`
        // intermediates we don't have to enumerate explicitly.
        walk_elixir_body(&child, bytes, out, depth + 1);
    }
}

fn first_elixir_arg_name(args: &Node, bytes: &[u8]) -> Option<String> {
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        match child.kind() {
            "alias" | "identifier" => {
                if let Ok(t) = child.utf8_text(bytes) {
                    return Some(t.to_string());
                }
            }
            // `def foo(x, y)` — the LHS is a `call` whose head identifier is
            // the function name.
            "call" => {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "identifier" {
                        if let Ok(t) = inner.utf8_text(bytes) {
                            return Some(t.to_string());
                        }
                    }
                }
            }
            // `def foo when ... do ... end` wraps into `binary_operator`.
            "binary_operator" => {
                return first_elixir_arg_name(&child, bytes);
            }
            _ => {}
        }
    }
    None
}

/// Header-only tree-sitter scan of an Erlang source file. Records every
/// top-level `function_declaration` name. Erlang also has `-record(...)`
/// and `-type(...)` attribute forms; record definitions are captured since
/// the chain walker references records as types.
fn scan_erlang_header(source: &str) -> Vec<String> {
    let language = tree_sitter_erlang::LANGUAGE.into();
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
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "fun_decl" | "function_declaration" | "function" => {
                // Erlang function clause head: `name(args) -> body.`
                // The first `atom` child is the function name.
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "atom" {
                        if let Ok(t) = inner.utf8_text(bytes) {
                            out.push(t.to_string());
                            break;
                        }
                    }
                    // clause-wrapped: walk one level down.
                    if inner.kind() == "function_clause" || inner.kind() == "clause" {
                        let mut cc = inner.walk();
                        for sub in inner.children(&mut cc) {
                            if sub.kind() == "atom" {
                                if let Ok(t) = sub.utf8_text(bytes) {
                                    out.push(t.to_string());
                                }
                                break;
                            }
                        }
                        break;
                    }
                }
            }
            "attribute" | "record_decl" | "type_alias" => {
                // `-record(name, {...}).` — second atom is the record name.
                let mut ic = child.walk();
                let mut seen_first_atom = false;
                for inner in child.children(&mut ic) {
                    if inner.kind() == "atom" {
                        if !seen_first_atom { seen_first_atom = true; continue }
                        if let Ok(t) = inner.utf8_text(bytes) {
                            out.push(t.to_string());
                        }
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Header-only tree-sitter scan of a Gleam source file. Records top-level
/// `pub fn`, `pub type`, `pub const`, plus their private counterparts.
fn scan_gleam_header(source: &str) -> Vec<String> {
    let language = tree_sitter_gleam::LANGUAGE.into();
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
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "function" | "constant" | "type_definition" | "external_function"
            | "external_type" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(t) = name_node.utf8_text(bytes) {
                        out.push(t.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "hex_tests.rs"]
mod tests;
