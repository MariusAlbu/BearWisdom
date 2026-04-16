// Erlang / rebar3 externals

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Erlang rebar3 → `discover_erlang_externals` + `walk_erlang_external_root`.
///
/// Discovery order (first hit wins per package):
///   1. `_build/default/lib/<name>/` — rebar3 already compiled; best case.
///   2. `~/.hex/packages/hexpm/<name>-<version>.tar` — hex package cache shared
///      with Elixir. Tarballs contain `contents.tar.gz` which holds the raw
///      Erlang source tree. Extraction is cached under
///      `~/.cache/bearwisdom/erlang-sources/<name>-<version>/`
///      (or `%LOCALAPPDATA%/bearwisdom/...` on Windows).
///
/// Version matching for hex tarballs:
///   - If `rebar.lock` is present, `parse_rebar_lock` extracts the pinned hex
///     version string for each dep. Used for an exact `<name>-<version>.tar`
///     probe.
///   - Without a lock file (or when the dep isn't in the lock), we scan
///     `~/.hex/packages/hexpm/<name>-*.tar` and pick the lexicographically
///     largest match (same strategy as Maven version fallback).
pub struct ErlangExternalsLocator;

impl ExternalSourceLocator for ErlangExternalsLocator {
    fn ecosystem(&self) -> &'static str { "erlang" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_erlang_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_erlang_external_root(dep)
    }
}

pub fn discover_erlang_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    // Require rebar.config as the manifest signal.
    let rebar_config = project_root.join("rebar.config");
    if !rebar_config.is_file() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&rebar_config) else {
        return Vec::new();
    };
    let declared = parse_rebar_deps(&content);
    if declared.is_empty() {
        return Vec::new();
    }

    // Authoritative versions from rebar.lock (may be absent on fresh clones).
    let locked_versions = parse_rebar_lock(project_root);

    // Path 1: _build/default/lib/ — populated by `rebar3 compile`.
    let build_lib = project_root.join("_build").join("default").join("lib");
    let build_available = build_lib.is_dir();

    // Path 2: hex package cache shared with Elixir — `~/.hex/packages/hexpm/`.
    let hex_cache = hex_packages_dir();

    let mut roots = Vec::new();

    for dep_name in &declared {
        // --- Path 1: _build ---
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
                    ecosystem: "erlang",
                    package_id: None,
                });
                continue;
            }
        }

        // --- Path 2: hex tarball ---
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
                    ecosystem: "erlang",
                    package_id: None,
                });
                continue;
            }
        }

        debug!("Erlang: dep '{dep_name}' not found in _build/ or hex cache — run `rebar3 compile` to populate");
    }

    debug!("Erlang: discovered {} external package roots", roots.len());
    roots
}

// ---------------------------------------------------------------------------
// rebar.lock parser — extracts pinned hex versions
// ---------------------------------------------------------------------------

/// Parse `rebar.lock` at `{project_root}/rebar.lock` and return a map of
/// dep_name → hex version string.
///
/// rebar.lock format (simplified):
/// ```text
/// {"1.2.0",
/// [{<<"cowlib">>,{pkg,<<"cowlib">>,<<"2.16.0">>,…},0},
///  {<<"ranch">>,{pkg,<<"ranch">>,<<"1.8.1">>,…},0}]}.
/// ```
///
/// We look for `{pkg,<<"<name>">>,<<"<version>">>` tuples and extract the
/// version. Non-hex deps (git, path) won't appear in this form and are
/// silently skipped — the hex fallback then does a version glob.
pub fn parse_rebar_lock(project_root: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let lock_path = project_root.join("rebar.lock");
    let Ok(content) = std::fs::read_to_string(&lock_path) else {
        return map;
    };
    // Scan for `{pkg,<<"<name>">>,<<"<version>">>`
    // We use a simple byte-scan: find `{pkg,` then extract the two
    // `<<"...">>` binary literals that follow.
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

/// Extract the content of an Erlang binary literal `<<"...">>` from the start
/// of `s`. Returns `(content, bytes_consumed)` or `None`.
fn read_binary_literal(s: &str) -> Option<(String, usize)> {
    let s_trimmed = s.trim_start();
    let rest = s_trimmed.strip_prefix("<<\"")?;
    let end = rest.find("\">>") ?;
    let leading = s.len() - s_trimmed.len();
    Some((rest[..end].to_string(), leading + 3 + end + 3))
}

// ---------------------------------------------------------------------------
// Hex package cache helpers
// ---------------------------------------------------------------------------

/// Return `~/.hex/packages/hexpm/` if the directory exists.
///
/// On Windows this is `%USERPROFILE%\.hex\packages\hexpm\`.
/// On Unix it is `~/.hex/packages/hexpm/`.
/// The `BEARWISDOM_HEX_PACKAGES` env var overrides for CI / non-standard setups.
fn hex_packages_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_HEX_PACKAGES") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".hex").join("packages").join("hexpm");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Persistent extraction cache for hex tarballs.
///
/// `~/.cache/bearwisdom/erlang-sources/` on Unix,
/// `%LOCALAPPDATA%\bearwisdom\erlang-sources\` on Windows.
/// `BEARWISDOM_ERLANG_SOURCE_CACHE` env var overrides.
fn erlang_source_cache_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_ERLANG_SOURCE_CACHE") {
        let p = PathBuf::from(explicit);
        std::fs::create_dir_all(&p).ok()?;
        return Some(p);
    }
    // Windows: %LOCALAPPDATA%\bearwisdom\erlang-sources
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = PathBuf::from(local)
            .join("bearwisdom")
            .join("erlang-sources");
        if std::fs::create_dir_all(&p).is_ok() {
            return Some(p);
        }
    }
    // Unix: ~/.cache/bearwisdom/erlang-sources
    if let Some(home) = std::env::var_os("HOME") {
        let p = PathBuf::from(home)
            .join(".cache")
            .join("bearwisdom")
            .join("erlang-sources");
        if std::fs::create_dir_all(&p).is_ok() {
            return Some(p);
        }
    }
    None
}

/// Find and extract the hex tarball for `dep_name`.
///
/// 1. If `pinned_version` is Some, probe `<cache>/<dep>-<version>.tar` exactly.
/// 2. Otherwise, glob `<cache>/<dep>-*.tar` and pick the lexicographically
///    largest match (same strategy as Maven version fallback).
/// 3. Extract `contents.tar.gz` from the hex tar into a persistent BearWisdom
///    source cache. Skip re-extraction when the cache dir is newer than the tar.
///
/// Returns `(version, extracted_dir)` or `None` on any failure.
fn locate_hex_dep(
    hex_cache: &Path,
    dep_name: &str,
    pinned_version: Option<&str>,
) -> Option<(String, PathBuf)> {
    let (tar_path, version) = if let Some(ver) = pinned_version {
        let p = hex_cache.join(format!("{dep_name}-{ver}.tar"));
        if p.is_file() {
            (p, ver.to_string())
        } else {
            return None;
        }
    } else {
        // Glob: find all `<dep_name>-*.tar` files and pick the largest.
        let entries = std::fs::read_dir(hex_cache).ok()?;
        let prefix = format!("{dep_name}-");
        let mut candidates: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let stripped = name.strip_prefix(&prefix)?.strip_suffix(".tar")?;
                let path = e.path();
                if path.is_file() {
                    Some((stripped.to_string(), path))
                } else {
                    None
                }
            })
            .collect();
        // Lexicographic sort — good enough for semver when patch < 10 per segment,
        // and exact when there's only one cached version.
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        let (ver, path) = candidates.into_iter().next_back()?;
        (path, ver)
    };

    // Determine the extraction target.
    let cache_base = erlang_source_cache_dir()?;
    let extracted = cache_base.join(format!("{dep_name}-{version}"));

    // Skip re-extraction if already done and not stale.
    if extracted.is_dir() && !is_hex_cache_stale(&tar_path, &extracted) {
        debug!("Erlang hex: using cached extraction for {dep_name}-{version}");
        return Some((version, extracted));
    }

    // Extract: open outer tar → find contents.tar.gz → decode gzip → unpack inner tar.
    match extract_hex_tarball(&tar_path, &extracted) {
        Ok(()) => {
            debug!(
                "Erlang hex: extracted {dep_name}-{version} to {}",
                extracted.display()
            );
            Some((version, extracted))
        }
        Err(e) => {
            debug!("Erlang hex: failed to extract {dep_name}-{version}: {e}");
            None
        }
    }
}

/// True when the hex `.tar` is newer than the newest file in `cache_dir`.
fn is_hex_cache_stale(tar: &Path, cache_dir: &Path) -> bool {
    let tar_mtime = match std::fs::metadata(tar).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return true;
    };
    let mut newest: Option<std::time::SystemTime> = None;
    for entry in entries.flatten() {
        if let Ok(md) = entry.metadata() {
            if let Ok(t) = md.modified() {
                newest = Some(newest.map(|cur| cur.max(t)).unwrap_or(t));
            }
        }
    }
    match newest {
        Some(t) => tar_mtime > t,
        None => true,
    }
}

/// Extract a hex `.tar` (POSIX tar) containing `contents.tar.gz` into `dest`.
///
/// Hex tarball layout:
/// ```text
/// VERSION
/// CHECKSUM
/// metadata.config
/// contents.tar.gz   ← the actual source tree
/// ```
///
/// We open the outer tar with the `tar` crate, locate `contents.tar.gz`,
/// stream it through `flate2::GzDecoder`, and unpack the inner tar into `dest`.
/// Only `.erl` and `.hrl` entries are written — build artifacts, docs, and
/// test suites are discarded. A zip-slip guard rejects any entry whose resolved
/// path escapes `dest`.
fn extract_hex_tarball(tar_path: &Path, dest: &Path) -> std::io::Result<()> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    std::fs::create_dir_all(dest)?;

    // Open the outer hex tar (uncompressed POSIX tar).
    let outer_file = std::fs::File::open(tar_path)?;
    let mut outer = ::tar::Archive::new(outer_file);

    for entry in outer.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let name = path.to_string_lossy();
        if name != "contents.tar.gz" {
            continue;
        }
        // Found the inner tarball — read it fully into memory then process.
        let mut gz_bytes = Vec::new();
        entry.read_to_end(&mut gz_bytes)?;

        let gz_cursor = std::io::Cursor::new(gz_bytes);
        let gz_decoder = GzDecoder::new(gz_cursor);
        let mut inner = ::tar::Archive::new(gz_decoder);

        for inner_entry in inner.entries()? {
            let mut inner_entry = inner_entry?;
            let inner_path = inner_entry.path()?.to_path_buf();

            let Some(file_name) = inner_path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // Only Erlang source + header files.
            if !(file_name.ends_with(".erl") || file_name.ends_with(".hrl")) {
                continue;
            }
            // Skip test suites inside the package source.
            if file_name.ends_with("_SUITE.erl") || file_name.ends_with("_tests.erl") {
                continue;
            }
            // Zip-slip guard: ensure the resolved output path stays inside dest.
            let out_path = dest.join(&inner_path);
            let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf());
            let canonical_out = match out_path.parent() {
                Some(parent) => {
                    if std::fs::create_dir_all(parent).is_err() {
                        continue;
                    }
                    parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf())
                }
                None => continue,
            };
            if !canonical_out.starts_with(&canonical_dest) {
                continue; // path traversal attempt — skip
            }

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
// Walker
// ---------------------------------------------------------------------------

pub fn walk_erlang_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    // Walk src/ and include/ — both produced by hex extraction and _build.
    for subdir in &["src", "include"] {
        let dir = dep.root.join(subdir);
        if dir.is_dir() {
            walk_erlang_dir(&dir, &dep.root, dep, &mut out);
        }
    }
    out
}

fn walk_erlang_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_erlang_dir_bounded(dir, root, dep, out, 0);
}

fn walk_erlang_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "examples" | "doc")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_erlang_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !(name.ends_with(".erl") || name.ends_with(".hrl")) {
                continue;
            }
            if name.ends_with("_SUITE.erl") || name.ends_with("_tests.erl") {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:erlang:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "erlang",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// rebar.config dep parser
// ---------------------------------------------------------------------------

/// Parse dep names from rebar.config `{deps, [...]}` section.
///
/// Handles both rebar3 hex-only form `{dep, "~> 1.0"}` and the older git
/// source form `{dep, ".*", {git, "url", {tag, "v1.0"}}}`. In both cases
/// the first atom inside each depth-1 brace-tuple is the dep name.
pub fn parse_rebar_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find("{deps,") else {
        return deps;
    };
    let rest = &content[start..];
    let Some(bracket_start) = rest.find('[') else {
        return deps;
    };
    let rest = &rest[bracket_start..];
    // Find the matching bracket, tracking nesting depth.
    let mut bracket_depth = 0i32;
    let mut bracket_end = None;
    for (i, ch) in rest.char_indices() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => {
                bracket_depth -= 1;
                if bracket_depth == 0 {
                    bracket_end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let bracket_end = match bracket_end {
        Some(e) => e,
        None => return deps,
    };
    let deps_block = &rest[1..bracket_end];

    // Top-level dep tuples: {atom, ...}. Track brace depth to only extract
    // the first atom of depth-1 tuples, skipping nested {git,...} etc.
    let mut brace_depth = 0u32;
    let mut in_atom = false;
    let mut atom_start = 0usize;
    for (i, ch) in deps_block.char_indices() {
        match ch {
            '{' => {
                brace_depth += 1;
                if brace_depth == 1 {
                    in_atom = true;
                    atom_start = i + 1;
                }
            }
            ',' | '}' if brace_depth == 1 && in_atom => {
                let name = deps_block[atom_start..i].trim();
                if !name.is_empty()
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    deps.push(name.to_string());
                }
                in_atom = false;
                if ch == '}' {
                    brace_depth = brace_depth.saturating_sub(1);
                }
            }
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    deps
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
        // rebar3 hex shorthand: {dep, "~> 1.0"}
        let content = r#"{deps, [{cowlib, "~> 2.12"}, {ranch, "~> 1.8"}]}."#;
        let deps = parse_rebar_deps(content);
        assert_eq!(deps, vec!["cowlib", "ranch"]);
    }

    #[test]
    fn erlang_parses_rebar_lock_versions() {
        let tmp = std::env::temp_dir().join("bw-test-erlang-rebar-lock");
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
        let tmp = std::env::temp_dir().join("bw-test-erlang-discover-build");
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

        // Suppress hex fallback so the test is deterministic regardless of what is
        // cached in the developer's ~/.hex/packages/hexpm/ directory.
        let empty_hex = tmp.join("empty-hex");
        std::fs::create_dir_all(&empty_hex).unwrap();
        std::env::set_var("BEARWISDOM_HEX_PACKAGES", &empty_hex);

        let roots = discover_erlang_externals(&tmp);

        std::env::remove_var("BEARWISDOM_HEX_PACKAGES");

        // Only cowlib is present in _build/; ranch is absent and hex cache is empty.
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].module_path, "cowlib");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn erlang_hex_fallback_extracts_erl_files() {
        // Build a minimal synthetic hex tarball in memory:
        // outer tar → contents.tar.gz → src/testpkg.erl + include/testpkg.hrl
        let tmp = std::env::temp_dir().join("bw-test-erlang-hex-extract");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let hex_cache = tmp.join("hex-cache");
        std::fs::create_dir_all(&hex_cache).unwrap();
        let source_cache = tmp.join("source-cache");

        // Build inner tar.gz (contents.tar.gz).
        let inner_gz = {
            let mut inner_buf = Vec::new();
            {
                let gz =
                    flate2::write::GzEncoder::new(&mut inner_buf, flate2::Compression::fast());
                let mut inner_tar = ::tar::Builder::new(gz);
                // src/testpkg.erl
                let src_data = b"-module(testpkg).\n-export([hello/0]).\nhello() -> world.\n";
                let mut header = ::tar::Header::new_gnu();
                header.set_path("src/testpkg.erl").unwrap();
                header.set_size(src_data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                inner_tar.append(&header, src_data.as_ref()).unwrap();
                // include/testpkg.hrl
                let hrl_data = b"-define(HELLO, world).\n";
                let mut hrl_hdr = ::tar::Header::new_gnu();
                hrl_hdr.set_path("include/testpkg.hrl").unwrap();
                hrl_hdr.set_size(hrl_data.len() as u64);
                hrl_hdr.set_mode(0o644);
                hrl_hdr.set_cksum();
                inner_tar.append(&hrl_hdr, hrl_data.as_ref()).unwrap();
                inner_tar.finish().unwrap();
            }
            inner_buf
        };

        // Build outer tar (uncompressed) with contents.tar.gz entry.
        let tar_path = hex_cache.join("testpkg-1.0.0.tar");
        {
            let tar_file = std::fs::File::create(&tar_path).unwrap();
            let mut outer_tar = ::tar::Builder::new(tar_file);
            let mut header = ::tar::Header::new_gnu();
            header.set_path("contents.tar.gz").unwrap();
            header.set_size(inner_gz.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            outer_tar.append(&header, inner_gz.as_slice()).unwrap();
            outer_tar.finish().unwrap();
        }

        // Project with rebar.config declaring testpkg (no _build/).
        let project = tmp.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("rebar.config"),
            r#"{deps, [{testpkg, "~> 1.0"}]}."#,
        )
        .unwrap();

        // Override env vars so the locator uses our synthetic dirs.
        std::env::set_var("BEARWISDOM_HEX_PACKAGES", &hex_cache);
        std::env::set_var("BEARWISDOM_ERLANG_SOURCE_CACHE", &source_cache);

        let roots = discover_erlang_externals(&project);

        std::env::remove_var("BEARWISDOM_HEX_PACKAGES");
        std::env::remove_var("BEARWISDOM_ERLANG_SOURCE_CACHE");

        assert_eq!(roots.len(), 1, "expected one root for testpkg");
        assert_eq!(roots[0].module_path, "testpkg");
        assert_eq!(roots[0].version, "1.0.0");

        let walked = walk_erlang_external_root(&roots[0]);
        let paths: Vec<&str> = walked.iter().map(|f| f.relative_path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.ends_with("src/testpkg.erl")),
            "expected src/testpkg.erl in walked files, got: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("include/testpkg.hrl")),
            "expected include/testpkg.hrl in walked files, got: {paths:?}"
        );
        assert!(paths.iter().all(|p| p.starts_with("ext:erlang:testpkg/")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn erlang_walk_skips_test_suites() {
        let tmp = std::env::temp_dir().join("bw-test-erlang-walk-skip");
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
            ecosystem: "erlang",
            package_id: None,
        };
        let walked = walk_erlang_external_root(&dep);
        assert_eq!(walked.len(), 1);
        assert!(walked[0].relative_path.ends_with("src/cowlib.erl"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
