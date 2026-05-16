// ---------------------------------------------------------------------------
// Discovery — union of three filesystem strategies
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use tracing::debug;

use super::reachability::collect_hex_user_imports;
use super::LEGACY_ECOSYSTEM_TAG;
use crate::ecosystem::externals::ExternalDepRoot;

pub(super) fn discover_hex_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
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

pub(super) fn discover_erlang_mk_roots(
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

pub(super) fn discover_mix_roots(project_root: &Path, user_imports: &[String]) -> Vec<ExternalDepRoot> {
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

pub(super) fn discover_rebar_roots(project_root: &Path, user_imports: &[String]) -> Vec<ExternalDepRoot> {
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

pub(super) fn discover_gleam_roots(project_root: &Path, user_imports: &[String]) -> Vec<ExternalDepRoot> {
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
