// =============================================================================
// ecosystem/vendored_self_declared.rs — self-declaring vendored-package discovery
//
// A vendored third-party package can ship its OWN package manifest inside the
// host tree — a `src/.../plugins/editor/package.json` (and/or `bower.json`)
// describing a foreign library, not a member of the host project. The manifest
// IS the declaration; this module reads it the same way `vendored_submodules`
// reads `.gitmodules`, classifying the subtree `origin='external'` so the host
// project's resolution rate isn't measured against vendored library internals.
//
// Conservative gate — fires only when the host project declares NONE of this
// ecosystem itself. A project that owns an npm package (a root `package.json`
// or an npm-family workspace) is never touched: a JS monorepo's members are
// self-declaring too, and partitioning them is ambiguous. The unambiguous case
// — a non-JS host (Java/Thymeleaf, Rails, etc.) carrying a vendored JS package —
// is the only one classified.
//
// Manifest-driven: the foreign `package.json` / `bower.json` files ARE the
// declaration. No path-segment guessing, no per-library name lists. Like
// `vendored_submodules`, the files already live in the project tree and are
// walked by the main scan; this module only supplies the external-classification
// signal consumed during origin assignment in `indexer/full.rs`.
//
// rebar3 (Erlang) extends the same shape with a different manifest family: a
// vendored dependency checkout carries its OWN `rebar.config` and/or
// `src/<pkg>.app.src`. The gate fires only when the HOST itself declares the
// rebar ecosystem (a root `rebar.config`), and never classifies a project app:
// rebar3 umbrella members live under the `apps/` and `lib/` app dirs, which are
// excluded structurally.
//
// Go vendoring and Composer (PHP) extend the same shape via an install ledger
// INSIDE a nested `vendor/` tree — see the `ledger` child module.
// =============================================================================

use std::path::Path;

use crate::types::PackageInfo;

mod ledger;

#[cfg(test)]
#[path = "vendored_self_declared_tests.rs"]
mod tests;

/// `workspace_kind` strings (from `detect_packages`) that mean the host project
/// owns the npm ecosystem as a workspace — its member `package.json` files are
/// first-party, never vendored.
const NPM_WORKSPACE_KINDS: &[&str] = &[
    "npm-workspaces",
    "pnpm-workspace",
    "turborepo",
    "lerna",
    "nx",
];

/// Vendored self-declared package subtree prefixes for the project at
/// `project_root`, normalized to forward slashes with surrounding slashes
/// trimmed. Each manifest family is gated independently: npm/bower subtrees are
/// reported only when the host owns NO npm ecosystem (conservative gate);
/// rebar3 subtrees only when the host declares a root `rebar.config`; go-vendor
/// and composer-vendor subtrees when a `vendor/` tree carries its ecosystem's
/// in-subtree install ledger. Empty when no gate fires or no vendored foreign
/// package is declared.
///
/// `packages` and `workspace_kind` are the output of `detect_packages` — the
/// recursive manifest scan already registered every in-tree `package.json` as
/// an npm `PackageInfo`, including deep vendored ones. This function partitions
/// those by the gate rather than re-walking for them. The rebar pass is a
/// bounded filesystem probe (erlang manifests are not in `detect_packages`).
pub fn self_declared_vendor_prefixes(
    project_root: &Path,
    packages: &[PackageInfo],
    workspace_kind: Option<&str>,
) -> Vec<String> {
    let mut out = if host_owns_npm(project_root, packages, workspace_kind) {
        Vec::new()
    } else {
        let mut out = npm_vendored_prefixes(packages);
        out.extend(bower_only_vendored_prefixes(project_root, &out));
        out
    };
    out.extend(rebar_vendored_prefixes(project_root, &out));
    out.extend(ledger::go_vendor_prefixes(project_root, &out));
    out.extend(ledger::composer_vendor_prefixes(project_root, &out));
    out
}

/// True when the host project declares the npm ecosystem itself: a root-level
/// `package.json` on disk, a recorded root npm package row, or an npm-family
/// workspace. Any one makes vendored/first-party partitioning ambiguous, so
/// the gate declines.
///
/// The on-disk probe is the structural signal — it reads the host's own
/// manifest declaration, the exact fact this gate is defined over. The two
/// derived signals can each be absent on a polyglot monorepo: `workspace_kind`
/// is single-valued and another ecosystem's root manifest can win detection,
/// and the root npm row can be excluded from `packages` as a workspace
/// controller (see `stage_discover`). Neither absence means the host owns no
/// npm.
fn host_owns_npm(
    project_root: &Path,
    packages: &[PackageInfo],
    workspace_kind: Option<&str>,
) -> bool {
    if project_root.join("package.json").is_file() {
        return true;
    }
    if workspace_kind.is_some_and(|k| NPM_WORKSPACE_KINDS.contains(&k)) {
        return true;
    }
    packages
        .iter()
        .any(|p| p.kind.as_deref() == Some("npm") && is_root_path(&p.path))
}

/// True for the project-root manifest path. `detect_packages` emits the root as
/// an empty string or `"."`.
fn is_root_path(path: &str) -> bool {
    path.is_empty() || path == "."
}

/// Every detected npm package's subtree path, normalized, restricted to
/// subtrees ≥ 2 path segments deep (`a/b`). A depth-1 `frontend/package.json`
/// is a plausible first-party SPA subproject of a non-JS host, not a vendored
/// library — the deeper a self-declared foreign manifest sits, the less
/// ambiguous it is. Reached only past the gate, so the host owns no npm.
fn npm_vendored_prefixes(packages: &[PackageInfo]) -> Vec<String> {
    packages
        .iter()
        .filter(|p| p.kind.as_deref() == Some("npm"))
        .map(|p| p.path.replace('\\', "/"))
        .map(|p| p.trim_matches('/').to_string())
        .filter(|p| is_deep_subtree(p))
        .collect()
}

/// True when `rel_path` is ≥ 2 path segments deep (contains an interior `/`).
fn is_deep_subtree(rel_path: &str) -> bool {
    rel_path.contains('/')
}

/// Vendored subtrees declared by a `bower.json` with NO sibling `package.json`.
/// `package.json`-bearing subtrees are already covered by `npm_vendored_prefixes`
/// (the manifest scan registers them); a `bower.json`-only subtree is invisible
/// to that scan, so it is discovered here. Bounded depth-2-or-deeper walk;
/// `already` (the npm prefixes) are skipped to avoid duplicate entries.
fn bower_only_vendored_prefixes(project_root: &Path, already: &[String]) -> Vec<String> {
    const MAX_DEPTH: u32 = 8;
    let mut out = Vec::new();
    walk_for_bower(project_root, project_root, 0, MAX_DEPTH, already, &mut out);
    out
}

/// Recursive helper for `bower_only_vendored_prefixes`. Prunes dotted dirs and
/// `node_modules` (already external). Registers a directory at depth ≥ 2 whose
/// `bower.json` has no sibling `package.json` and is not already an npm prefix.
fn walk_for_bower(
    project_root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    already: &[String],
    out: &mut Vec<String>,
) {
    if depth > max_depth {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    let mut has_bower = false;
    let mut has_package = false;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if ft.is_dir() {
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            subdirs.push(entry.path());
        } else if ft.is_file() {
            match name.as_str() {
                "bower.json" => has_bower = true,
                "package.json" => has_package = true,
                _ => {}
            }
        }
    }

    // Depth ≥ 2 guard: a root or first-level `bower.json` is the host's own,
    // not a vendored subtree. (`project_root` itself is depth 0.)
    if has_bower && !has_package && depth >= 2 {
        if let Ok(rel) = dir.strip_prefix(project_root) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            let rel = rel.trim_matches('/').to_string();
            if !rel.is_empty() && !already.iter().any(|p| p == &rel) {
                out.push(rel);
            }
        }
    }

    for sub in subdirs {
        walk_for_bower(project_root, &sub, depth + 1, max_depth, already, out);
    }
}

/// rebar3 default `project_app_dirs` — the top-level path segments under which
/// umbrella PROJECT applications live. A self-declaring subtree under one of
/// these is a first-party app, never a vendored dependency. `rebar.config` may
/// override `project_app_dirs`, but the reader does not parse that key, so these
/// defaults are the conservative protection gate.
const REBAR_APP_DIRS: &[&str] = &["apps", "lib"];

/// rebar3 vendored-dependency subtree prefixes for the project at
/// `project_root`. Empty unless the HOST declares the rebar ecosystem (a root
/// `rebar.config`). A subtree is vendored when it self-declares an erlang
/// application (its own `rebar.config` or a `src/*.app.src`) AND its top path
/// segment is not a rebar app dir (`apps/`, `lib/`) — i.e. it is not a project
/// umbrella member. `already` (the npm/bower prefixes) are skipped to avoid
/// duplicate entries.
fn rebar_vendored_prefixes(project_root: &Path, already: &[String]) -> Vec<String> {
    if !host_declares_rebar(project_root) {
        return Vec::new();
    }
    const MAX_DEPTH: u32 = 8;
    let mut out = Vec::new();
    walk_for_rebar_app(project_root, project_root, 0, MAX_DEPTH, already, &mut out);
    out
}

/// True when `project_root` carries a top-level `rebar.config` — the host
/// declares the rebar ecosystem. The unambiguous signal that vendored erlang
/// checkouts inside the tree are foreign, not first-party.
fn host_declares_rebar(project_root: &Path) -> bool {
    project_root.join("rebar.config").is_file()
        || project_root.join("rebar3.config").is_file()
}

/// Recursive helper for `rebar_vendored_prefixes`. Prunes dotted dirs and the
/// rebar build-output `_build`. Registers a directory at depth ≥ 1 that
/// self-declares an erlang application and is not under a project app dir.
fn walk_for_rebar_app(
    project_root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    already: &[String],
    out: &mut Vec<String>,
) {
    if depth > max_depth {
        return;
    }

    // Depth ≥ 1: the host root self-declares its OWN application; only nested
    // subtrees are candidate vendored checkouts.
    if depth >= 1 && declares_erlang_app(dir) {
        if let Ok(rel) = dir.strip_prefix(project_root) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            let rel = rel.trim_matches('/').to_string();
            // Project app dirs (`apps/<x>`, `lib/<x>`) are first-party umbrella
            // members — never vendored.
            let in_app_dir = rel
                .split('/')
                .next()
                .is_some_and(|seg| REBAR_APP_DIRS.contains(&seg));
            if !rel.is_empty() && !in_app_dir && !already.iter().any(|p| p == &rel) {
                // A self-declaring vendored app is a leaf — don't descend into
                // its own nested deps, which the prefix already covers.
                out.push(rel);
                return;
            }
        }
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "_build" {
            continue;
        }
        walk_for_rebar_app(project_root, &entry.path(), depth + 1, max_depth, already, out);
    }
}

/// True when `dir` self-declares an erlang/OTP application: a `rebar.config`
/// at the subtree root, or a `src/*.app.src` application resource file.
fn declares_erlang_app(dir: &Path) -> bool {
    if dir.join("rebar.config").is_file() || dir.join("rebar3.config").is_file() {
        return true;
    }
    let src = dir.join("src");
    let Ok(entries) = std::fs::read_dir(&src) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .ends_with(".app.src")
    })
}

/// True when `rel_path` (project-root-relative) falls inside one of the
/// vendored self-declared subtrees. Matches the subtree exactly or any
/// descendant — a prefix sibling (`editor` vs `editormd/x`) does NOT match.
pub fn is_under_self_declared_vendor(rel_path: &str, prefixes: &[String]) -> bool {
    if prefixes.is_empty() {
        return false;
    }
    let norm = rel_path.replace('\\', "/");
    prefixes.iter().any(|p| {
        norm == *p
            || norm
                .strip_prefix(p.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}
