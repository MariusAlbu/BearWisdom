//! Demand-driven script-tag dependency parser.
//!
//! After the main parse pass, scan host-language files (HTML, Razor,
//! cshtml, Vue, Svelte, Astro, etc.) for `<script src="…">` `Imports` refs
//! emitted by their extractors, resolve the referenced URLs to filesystem
//! paths, and parse those vendor files as `origin='external'`.
//!
//! This is the demand-driven counterpart to the per-ecosystem
//! externals pipeline (npm / nuget / go / maven / pypi): instead of
//! eagerly walking `node_modules/` or `wwwroot/lib/`, we only pull in
//! files that some host file in the project actually references.
//!
//! The walker's `is_vendor_lib_dir` exclusion stays in force — that
//! blanket exclusion prevented indexing every `.min.js` in the tree and
//! the design intent stands. This stage injects back only the specific
//! files that `<script src>` tags name, bypassing the exclusion on a
//! per-file basis.
//!
//! URL resolution:
//!   * `~/foo/bar.js`   → `{webroot}/foo/bar.js`
//!   * `/foo/bar.js`    → `{webroot}/foo/bar.js`
//!   * relative `foo.js` → resolved against host file's directory
//!
//! `{webroot}` is discovered by walking up from the host file until a
//! `wwwroot/`, `public/`, `static/`, or `web/` sibling is found.
//!
//! Absolute URLs (`http://…`, `//cdn.example.com/…`, `data:…`) are filtered
//! out at extraction time and never reach this stage.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::indexer::full::parse_file;
use crate::languages::registry::LanguageRegistry;
use crate::types::{EdgeKind, ParsedFile};
use crate::walker::{detect_language, WalkedFile};

/// Scan `parsed` for `<script src>` references emitted by host-language
/// extractors and return `ParsedFile`s for each resolved vendor file.
///
/// Returned files carry language-detected `ParsedFile.language` and
/// preserve the absolute-path-derived relative_path for later write-pass
/// consumption. The caller writes them with `origin='external'`.
pub fn parse_script_tag_deps(
    project_root: &Path,
    parsed: &[ParsedFile],
    registry: &LanguageRegistry,
) -> Vec<ParsedFile> {
    let mut refs: Vec<(&str, &str)> = Vec::new();
    let existing_paths: HashSet<&str> =
        parsed.iter().map(|pf| pf.path.as_str()).collect();

    for pf in parsed {
        if !is_script_host_language(&pf.language) {
            continue;
        }
        for r in &pf.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let Some(module) = r.module.as_deref() else {
                continue;
            };
            if looks_like_script_tag_url(module) {
                refs.push((pf.path.as_str(), module));
            }
        }
    }

    if refs.is_empty() {
        return Vec::new();
    }

    // Resolve and dedupe.
    let mut resolved: HashSet<PathBuf> = HashSet::new();
    for (host_path, url) in &refs {
        if let Some(abs) = resolve_script_url(project_root, host_path, url) {
            resolved.insert(abs);
        }
    }

    let mut out = Vec::with_capacity(resolved.len());
    for abs in resolved {
        let rel = match abs.strip_prefix(project_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue, // Outside project root — skip.
        };
        if existing_paths.contains(rel.as_str()) {
            // Already parsed as part of the regular walk — don't double-parse.
            continue;
        }
        let Some(lang) = detect_language(&abs) else {
            continue;
        };
        let walked = WalkedFile {
            relative_path: rel.clone(),
            absolute_path: abs.clone(),
            language: lang,
        };
        match parse_file(&walked, registry) {
            Ok(pf) => {
                debug!("script-tag dep: parsed {} as {}", rel, lang);
                out.push(pf);
            }
            Err(e) => warn!("script-tag dep parse failed for {}: {e}", rel),
        }
    }
    out
}

/// Host languages that can emit `<script src>` refs.
fn is_script_host_language(lang: &str) -> bool {
    matches!(
        lang,
        "html" | "razor" | "cshtml" | "blade" | "erb" | "vue" | "svelte" | "astro"
    )
}

/// True when a ref module string is shaped like a script-tag URL the
/// resolver should follow. Accepts `~/…`, `/…`, and relative paths with
/// a web-asset extension.
fn looks_like_script_tag_url(module: &str) -> bool {
    if module.starts_with("~/") || module.starts_with('/') {
        return true;
    }
    // Relative path — require a recognized web-asset extension so we
    // don't accidentally supplementary-parse arbitrary `Imports` refs
    // from other extractors (markdown link refs share the same kind).
    has_web_asset_extension(module)
}

fn has_web_asset_extension(url: &str) -> bool {
    let path = url.split(&['?', '#'][..]).next().unwrap_or(url);
    let Some(dot) = path.rfind('.') else {
        return false;
    };
    let ext = &path[dot + 1..];
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "js" | "mjs" | "cjs" | "ts" | "tsx" | "css" | "scss" | "less"
    )
}

/// Resolve a script URL against the webroot (for `~/` and `/` prefixes)
/// or the host file's directory (for relative paths).
fn resolve_script_url(
    project_root: &Path,
    host_file_path: &str,
    url: &str,
) -> Option<PathBuf> {
    let clean_url = url.split(&['?', '#'][..]).next()?.trim();
    if clean_url.is_empty() {
        return None;
    }

    let host_abs = project_root.join(host_file_path);

    let rest_opt = clean_url.strip_prefix("~/").or_else(|| clean_url.strip_prefix('/'));
    if let Some(rest) = rest_opt {
        // ASP.NET modular layouts expose TWO classes of webroot for a given
        // URL: the module's own `Modules/Foo/wwwroot/` (module-local assets
        // — `main.js`, theme CSS) and the WebHost's shared
        // `WebHost/wwwroot/` (vendor libs — `lib/jquery/jquery.js`,
        // `lib/ng-file-upload/dist/…`). Ancestor-walking finds the
        // module's webroot first, but the URL the host .cshtml references
        // lives in the sibling WebHost. Try every discovered webroot and
        // return the first that resolves.
        //
        // ASP.NET Core static-web-assets convention: `~/_content/{pkg}/foo`
        // maps to `{pkg}/wwwroot/foo` where `{pkg}` is the project/assembly
        // name. Strip `_content/{pkg}/` and scan every module subdirectory
        // whose name matches `{pkg}` for a `wwwroot/foo` file.
        if let Some(rest_of_content) = rest.strip_prefix("_content/") {
            if let Some(slash) = rest_of_content.find('/') {
                let pkg = &rest_of_content[..slash];
                let sub = &rest_of_content[slash + 1..];
                if let Some(p) = find_static_web_asset(project_root, pkg, sub) {
                    return Some(p);
                }
            }
        }
        for webroot in find_webroots_for_host(project_root, &host_abs) {
            let candidate = webroot.join(rest);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    } else {
        let candidate = host_abs.parent()?.join(clean_url);
        if candidate.is_file() {
            Some(candidate)
        } else {
            None
        }
    }
}

/// Resolve an ASP.NET Core `_content/{pkg}/sub/path` URL to an absolute
/// filesystem path.
///
/// Scans every directory under `project_root` (two levels deep) looking
/// for one whose filename matches `pkg`. When a match is found, appends
/// `wwwroot/{sub}` and checks the file exists. Returns the first match —
/// the Razor Class Library convention pairs each `{pkg}` with exactly one
/// project directory.
fn find_static_web_asset(project_root: &Path, pkg: &str, sub: &str) -> Option<PathBuf> {
    let try_at = |dir: &Path| -> Option<PathBuf> {
        let candidate = dir.join("wwwroot").join(sub);
        if candidate.is_file() {
            Some(candidate)
        } else {
            None
        }
    };
    let entries = std::fs::read_dir(project_root).ok()?;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() { continue }
        let d = entry.path();
        if d.file_name().and_then(|n| n.to_str()) == Some(pkg) {
            if let Some(p) = try_at(&d) { return Some(p); }
        }
        // Nested scan: modular layouts put projects one level deeper
        // (`src/Modules/{pkg}/wwwroot/`).
        if let Ok(sub_entries) = std::fs::read_dir(&d) {
            for sub_entry in sub_entries.flatten() {
                let Ok(sft) = sub_entry.file_type() else { continue };
                if !sft.is_dir() { continue }
                let dd = sub_entry.path();
                if dd.file_name().and_then(|n| n.to_str()) == Some(pkg) {
                    if let Some(p) = try_at(&dd) { return Some(p); }
                }
            }
        }
    }
    None
}

/// Return every webroot visible to a host file, most-specific first.
///
/// ASP.NET modular solutions serve assets from multiple webroots
/// simultaneously: each module's own `Modules/Foo/wwwroot/` (local assets)
/// AND the host project's shared `WebHost/wwwroot/` (vendor libs). A `~/…`
/// URL in a module .cshtml can land in either. Walk ancestors for
/// module-local webroots, then scan siblings under the project root for
/// shared webroots — callers try each in order until the URL resolves.
///
/// Candidate names are tried in specificity order: `wwwroot` (the ASP.NET
/// Core convention) first, then `public`/`static`/`web` as broader
/// fallbacks. The two-pass ordering prevents a random `Infrastructure/web/`
/// test-fixture directory from winning over the real `WebHost/wwwroot/`.
fn find_webroots_for_host(project_root: &Path, host_abs: &Path) -> Vec<PathBuf> {
    const CANDIDATES: &[&str] = &["wwwroot", "public", "static", "web"];
    let mut out: Vec<PathBuf> = Vec::new();
    let push_unique = |out: &mut Vec<PathBuf>, p: PathBuf| {
        if !out.iter().any(|e| e == &p) {
            out.push(p);
        }
    };
    // Ancestor walk — captures module-local webroots first (most specific
    // to the host file's own tree).
    let mut cur_opt = host_abs.parent();
    while let Some(cur) = cur_opt {
        for name in CANDIDATES {
            let p = cur.join(name);
            if p.is_dir() {
                push_unique(&mut out, p);
            }
        }
        if cur == project_root {
            break;
        }
        cur_opt = match cur.parent() {
            Some(p) if p.starts_with(project_root) || p == project_root => Some(p),
            _ => None,
        };
    }
    // Sibling scan — captures shared webroots in sibling projects
    // (e.g. `src/WebHost/wwwroot/` from a module under `src/Modules/`).
    // Two levels deep: `project_root/*/wwwroot` AND `project_root/*/*/wwwroot`.
    let scan_dir = |dir: &Path, name: &str, out: &mut Vec<PathBuf>| {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_dir() { continue }
            let sibling = entry.path();
            let p = sibling.join(name);
            if p.is_dir() { push_unique(out, p); }
            if let Ok(sub) = std::fs::read_dir(&sibling) {
                for sub_entry in sub.flatten() {
                    let Ok(sft) = sub_entry.file_type() else { continue };
                    if !sft.is_dir() { continue }
                    let p = sub_entry.path().join(name);
                    if p.is_dir() { push_unique(out, p); }
                }
            }
        }
    };
    for name in CANDIDATES {
        scan_dir(project_root, name, &mut out);
    }
    out
}

#[cfg(test)]
#[path = "script_tag_deps_tests.rs"]
mod tests;
