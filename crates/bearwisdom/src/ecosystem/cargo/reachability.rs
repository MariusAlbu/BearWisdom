// ---------------------------------------------------------------------------
// Reachability: crate entry + bounded `mod X;` expansion
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

use super::features::enabled_features_for_root;

const RS_MOD_MAX_DEPTH: u32 = 3;

/// Module-graph depth bound used when a crate is feature-gated. Cargo feature
/// names encode the module path (`Win32_Graphics_Dwm` → `Win32/Graphics/Dwm`),
/// so a reachable gated module can sit several levels below the crate entry.
/// The cfg gate caps breadth (only enabled feature subtrees are descended), so
/// raising the depth bound here cannot reintroduce the unbounded breadth that
/// the eager filesystem walk produced — it only lets a reached feature's
/// defining file stay locatable.
const RS_MOD_GATED_MAX_DEPTH: u32 = 8;

/// One `mod`/`pub mod` declaration plus the `#[cfg(feature = "X")]` attribute
/// (if any) that immediately precedes it. `cfg_feature` is `None` when the
/// declaration carries no feature gate — those modules are always reachable.
#[derive(Debug, Clone)]
pub(super) struct ModDecl {
    pub(super) cfg_feature: Option<String>,
    pub(super) name: String,
}

/// Start from `src/lib.rs` (falling back to `src/main.rs` for binary-only
/// crates) and recursively follow `mod X;` declarations into the matching
/// source files. Each file yields zero or more child modules; bounded at
/// depth 3 so deeply nested crates still get their top surface without
/// walking every internal module.
pub(super) fn resolve_crate_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    // Crates with a non-standard layout (`tree-sitter`'s
    // `binding_rust/lib.rs`, vendored embedded bindings, etc.) declare
    // their entry via `[lib] path = "..."` in Cargo.toml. Honor that
    // first; fall back to the conventional `src/lib.rs` / `src/main.rs`.
    // Without this branch, ~6-10% of cargo deps in a typical workspace
    // walk to zero files (every C-with-Rust-bindings crate, every crate
    // that carves up its workspace into custom directories).
    let entry = lib_entry_from_manifest(&dep.root).or_else(|| {
        let src = dep.root.join("src");
        let lib = src.join("lib.rs");
        if lib.is_file() {
            Some(lib)
        } else {
            let main = src.join("main.rs");
            if main.is_file() {
                Some(main)
            } else {
                None
            }
        }
    });
    let Some(entry) = entry else {
        return Vec::new();
    };
    if !entry.is_file() {
        return Vec::new();
    }

    let enabled = enabled_features_for_root(&dep.root);
    let max_depth = if enabled.is_empty() {
        RS_MOD_MAX_DEPTH
    } else {
        RS_MOD_GATED_MAX_DEPTH
    };
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    expand_rust_mods_into(dep, &dep.root, &entry, &enabled, &mut out, &mut seen, 0, max_depth);
    out
}

/// Read `[lib] path = "..."` from a crate's Cargo.toml, returning the
/// resolved absolute path if the file exists. Cheap text scan — no toml
/// parser dependency, no allocation beyond the read string. Tolerates
/// the field appearing under either `[lib]` or `[package.metadata]`-style
/// tables; we only look for the literal `[lib]` table since that's where
/// crates publishing on crates.io put it.
pub(super) fn lib_entry_from_manifest(crate_root: &Path) -> Option<PathBuf> {
    let manifest = crate_root.join("Cargo.toml");
    let content = std::fs::read_to_string(&manifest).ok()?;
    let mut in_lib = false;
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            // Match `[lib]` exactly — not `[lib.something]` and not
            // `[[bin]]`. Anything else exits the [lib] table.
            in_lib = line == "[lib]";
            continue;
        }
        if !in_lib {
            continue;
        }
        let Some(stripped) = line.strip_prefix("path") else {
            continue;
        };
        let stripped = stripped.trim_start();
        let Some(rest) = stripped.strip_prefix('=') else {
            continue;
        };
        let val = rest.trim();
        // Strip surrounding quotes.
        let Some(val) = val
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| val.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        else {
            continue;
        };
        let abs = crate_root.join(val);
        if abs.is_file() {
            return Some(abs);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn expand_rust_mods_into(
    dep: &ExternalDepRoot,
    crate_root: &Path,
    file: &Path,
    enabled: &[String],
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
    max_depth: u32,
) {
    if !seen.insert(file.to_path_buf()) {
        return;
    }
    if !file.is_file() {
        return;
    }

    let rel_sub = match file.strip_prefix(crate_root) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mod.rs")
            .to_string(),
    };
    out.push(WalkedFile {
        relative_path: format!("ext:rust:{}/{}", dep.module_path, rel_sub),
        absolute_path: file.to_path_buf(),
        language: "rust",
    });

    if depth >= max_depth {
        return;
    }

    let Ok(src) = std::fs::read_to_string(file) else {
        return;
    };

    // `include!("Windows/mod.rs")` pastes content textually — its `mod`/`pub
    // use` declarations resolve relative to the included file's own location,
    // and it does not consume a module nesting level. The `windows` crate's
    // entire API surface is reached this way from `lib.rs`.
    for inc in extract_rust_include_paths(&src) {
        let Some(next) = resolve_rust_include_path(file, &inc) else {
            continue;
        };
        expand_rust_mods_into(dep, crate_root, &next, enabled, out, seen, depth, max_depth);
    }

    for decl in extract_rust_mod_decls_with_cfg(&src) {
        // FAIL OPEN: a `pub mod` with no cfg attr, or one gated on a feature
        // we can't resolve as disabled, is always descended. The gate only
        // drops a module when the crate has an explicit enabled-feature set
        // AND the module's cfg feature is provably not in it.
        if !cfg_mod_reachable(decl.cfg_feature.as_deref(), enabled) {
            continue;
        }
        let Some(next) = resolve_rust_mod_path(file, &decl.name) else {
            continue;
        };
        expand_rust_mods_into(dep, crate_root, &next, enabled, out, seen, depth + 1, max_depth);
    }

    // Follow intra-crate `pub use crate::a::b::Type;` / `pub use self::x;`
    // re-exports so a type re-exported into an enabled module stays locatable
    // (the type-hop closure). Only module-path components are followed; the
    // walked file enters `out` and its own children expand under the same gate.
    for module_path in extract_rust_pub_use_modules(&src) {
        for next in resolve_rust_use_module_files(crate_root, file, &module_path) {
            expand_rust_mods_into(dep, crate_root, &next, enabled, out, seen, depth + 1, max_depth);
        }
    }
}

/// Decide whether a `#[cfg(feature = "F")] pub mod ...;` should be descended.
///
/// Returns `true` (descend) when:
///   * the declaration has no feature gate (`cfg_feature` is `None`), or
///   * `enabled` is empty — no resolved feature set, so we FAIL OPEN and walk
///     everything (preserves the pre-gate behaviour for crates whose features
///     we can't read), or
///   * `F` is enabled. `F` counts as enabled when an enabled feature equals
///     `F` exactly, or is a descendant of `F` in cargo's underscore-delimited
///     feature hierarchy (`Win32_Foundation` enables its prerequisite `Win32`,
///     so a module gated on `Win32` stays reachable). Superset matching is
///     intentional: over-gating (dropping a reached module) is a regression,
///     over-pulling is safe.
pub(super) fn cfg_mod_reachable(cfg_feature: Option<&str>, enabled: &[String]) -> bool {
    let Some(feature) = cfg_feature else {
        return true;
    };
    if enabled.is_empty() {
        return true;
    }
    enabled.iter().any(|e| {
        e == feature
            || e.strip_prefix(feature)
                .map_or(false, |rest| rest.starts_with('_'))
    })
}

/// Scan line-oriented for `mod X;` and `pub mod X;` (including
/// `pub(crate) mod X;` and similar visibility modifiers), reporting the
/// `#[cfg(feature = "X")]` attribute on the line *immediately* preceding each
/// declaration. The `windows` crate places exactly one such attribute on its
/// own line above each generated `pub mod`, so a single line of look-back is
/// sufficient. Inline `mod X { ... }` bodies are skipped — their contents are
/// already in the same file. Lines that are neither a recognized mod decl nor
/// a blank/attribute line reset the pending cfg so it never leaks onto an
/// unrelated later decl.
pub(super) fn extract_rust_mod_decls_with_cfg(src: &str) -> Vec<ModDecl> {
    let mut out = Vec::new();
    let mut pending_cfg: Option<String> = None;
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(feature) = parse_cfg_feature_attr(line) {
            pending_cfg = Some(feature);
            continue;
        }
        if let Some(name) = parse_mod_decl_line(raw) {
            out.push(ModDecl {
                cfg_feature: pending_cfg.take(),
                name,
            });
            continue;
        }
        // Any other content (a different attribute, a `use`, etc.) breaks the
        // attribute/decl adjacency — drop the pending cfg.
        pending_cfg = None;
    }
    out
}

/// Parse one `[mod | pub mod | pub(...) mod] X;` line, returning the module
/// identifier. Inline `mod X { ... }` bodies (no trailing `;`) yield `None`.
fn parse_mod_decl_line(raw: &str) -> Option<String> {
    let line = raw.trim_start();
    let line = match line.find("//") {
        Some(ix) => &line[..ix],
        None => line,
    };
    let line = line.trim();
    if !line.ends_with(';') {
        return None;
    }

    let mut rest = line;
    if let Some(r) = rest.strip_prefix("pub") {
        rest = r.trim_start();
        // Optional visibility qualifier: pub(crate), pub(super), pub(in path)
        if let Some(r) = rest.strip_prefix('(') {
            let close = r.find(')')?;
            rest = r[close + 1..].trim_start();
        }
    }

    let r = rest.strip_prefix("mod")?;
    // Ensure the next char is whitespace — avoid matching `model;` etc.
    if !r.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let ident = r.trim_start().trim_end_matches(';').trim();
    if ident.is_empty() || !ident.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(ident.to_string())
}

/// Recognize `#[cfg(feature = "X")]` on its own line, returning `X`. Only the
/// single-condition form is decoded; `all(...)`/`any(...)`/`not(...)` wrappers
/// return `None`, which makes the gated module FAIL OPEN (always walked).
fn parse_cfg_feature_attr(line: &str) -> Option<String> {
    let inner = line
        .strip_prefix("#[cfg(")
        .and_then(|s| s.strip_suffix(")]"))?
        .trim();
    let rest = inner.strip_prefix("feature")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let feature = &rest[..end];
    if feature.is_empty() {
        None
    } else {
        Some(feature.to_string())
    }
}

/// Recognize `include!("relative/path.rs");` lines, returning the path string.
fn extract_rust_include_paths(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("include!(") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.find('"') else {
            continue;
        };
        let path = &rest[..end];
        if !path.is_empty() {
            out.push(path.to_string());
        }
    }
    out
}

/// Resolve an `include!("...")` argument relative to the including file's
/// directory. `include!` paths are always relative to the source file.
fn resolve_rust_include_path(from_file: &Path, rel: &str) -> Option<PathBuf> {
    let parent = from_file.parent()?;
    let abs = parent.join(rel.replace('\\', "/"));
    abs.is_file().then_some(abs)
}

/// Extract the module-path prefix of intra-crate `pub use` re-exports —
/// `pub use crate::a::b::Type;` → `a::b`, `pub use self::x::Y;` → `x`. Returns
/// the path components leading up to (but excluding) the final re-exported
/// item. External re-exports (`pub use other_crate::...`), glob/group forms,
/// and `as`-renames are skipped: only the simple intra-crate single-item shape
/// is followed, which is all the type-hop closure needs.
fn extract_rust_pub_use_modules(src: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("pub use ") else {
            continue;
        };
        let Some(path) = rest.strip_suffix(';') else {
            continue;
        };
        let path = path.trim();
        // Group (`{...}`), glob (`*`), and rename (` as `) forms are not the
        // single-item shape we follow.
        if path.contains('{') || path.contains('*') || path.contains(" as ") {
            continue;
        }
        let segments: Vec<&str> = path.split("::").map(|s| s.trim()).collect();
        if segments.len() < 2 {
            continue;
        }
        // Anchor on an intra-crate root so external re-exports are ignored.
        let mods: Vec<String> = match segments[0] {
            "crate" | "self" => segments[1..segments.len() - 1]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            "super" => continue,
            _ => continue,
        };
        if !mods.is_empty() {
            out.push(mods);
        }
    }
    out
}

/// Resolve the module-path components of a `pub use crate::a::b::Type;` to the
/// source files that define those modules, so a re-exported type stays
/// locatable. Resolution starts from the crate's `src/` directory (where
/// `crate::` is rooted) and walks each component as `<dir>/<seg>.rs` or
/// `<dir>/<seg>/mod.rs`. Returns every module file found along the chain; an
/// unresolved component stops the walk for that path.
fn resolve_rust_use_module_files(
    crate_root: &Path,
    from_file: &Path,
    mods: &[String],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // `crate::` rooting: the directory holding the crate entry. Fall back to
    // the including file's own directory when the entry isn't under `src/`.
    let mut dir = crate_root.join("src");
    if !dir.is_dir() {
        dir = from_file.parent().map(|p| p.to_path_buf()).unwrap_or(dir);
    }
    for seg in mods {
        let as_file = dir.join(format!("{seg}.rs"));
        let as_mod = dir.join(seg).join("mod.rs");
        if as_file.is_file() {
            out.push(as_file);
            dir = dir.join(seg);
        } else if as_mod.is_file() {
            out.push(as_mod);
            dir = dir.join(seg);
        } else {
            break;
        }
    }
    out
}

/// Resolve `mod X;` declared in `from_file` to either `X.rs` or `X/mod.rs`
/// relative to the owning module directory (file's parent for lib.rs/
/// main.rs/mod.rs; sibling directory named after the stem otherwise).
pub(super) fn resolve_rust_mod_path(from_file: &Path, child: &str) -> Option<PathBuf> {
    let parent = from_file.parent()?;
    let stem = from_file.file_stem().and_then(|s| s.to_str())?;
    let mod_dir = if stem == "lib" || stem == "main" || stem == "mod" {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    };

    let as_file = mod_dir.join(format!("{child}.rs"));
    if as_file.is_file() {
        return Some(as_file);
    }
    let as_mod = mod_dir.join(child).join("mod.rs");
    if as_mod.is_file() {
        return Some(as_mod);
    }
    None
}
