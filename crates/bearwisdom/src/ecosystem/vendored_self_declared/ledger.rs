// =============================================================================
// ecosystem/vendored_self_declared/ledger.rs — vendor-directory install ledgers
//
// Go vendoring and Composer (PHP) self-declare a vendored subtree via an
// install ledger INSIDE a `vendor/` directory. A root-level `vendor/` is
// already dropped by the file walker (`ROOT_ONLY_EXCLUDE_NAMES`), so the
// residual surface is a NESTED `vendor/` — a monorepo package's
// `services/api/vendor/...`, which the walker does index. Such a tree
// self-declares as external via a manifest INSIDE it:
//   * go-vendor: `vendor/modules.txt` (Go's vendor inventory) at the subtree
//     root, optionally with vendored modules carrying their own `go.mod`.
//   * composer-vendor: `vendor/composer/installed.json` (Composer's install
//     ledger) at the subtree root.
// The HOST's own module is protected: the project's own `go.mod` /
// `composer.json` is never under a `vendor/` segment, so it is never matched.
// =============================================================================

use std::path::Path;

/// Go-vendored subtree prefixes for the project at `project_root`. A `vendor/`
/// directory carrying Go's `vendor/modules.txt` inventory at its root is a Go
/// vendored-dependency tree; the whole `vendor/` subtree is third-party. The
/// project's own module (its root `go.mod`) is never under a `vendor/` segment,
/// so it is never matched. A root-level `vendor/` is already dropped by the file
/// walker — this catches the nested-monorepo-package case it does not. `already`
/// (prior prefixes) are skipped to avoid duplicate entries.
pub(super) fn go_vendor_prefixes(project_root: &Path, already: &[String]) -> Vec<String> {
    vendor_ledger_prefixes(project_root, already, go_vendor_ledger)
}

/// Composer-vendored subtree prefixes for the project at `project_root`. A
/// `vendor/` directory carrying Composer's `vendor/composer/installed.json`
/// ledger is a Composer vendored-dependency tree; the whole `vendor/` subtree is
/// third-party. The project's own package (its root `composer.json`) is never
/// under a `vendor/` segment, so it is never matched. A root-level `vendor/` is
/// already dropped by the file walker — this catches the nested case it does
/// not. `already` (prior prefixes) are skipped to avoid duplicate entries.
pub(super) fn composer_vendor_prefixes(project_root: &Path, already: &[String]) -> Vec<String> {
    vendor_ledger_prefixes(project_root, already, composer_vendor_ledger)
}

/// True when `dir` is a Go vendored-dependency tree: it carries the
/// `modules.txt` inventory `go mod vendor` writes at the `vendor/` root.
fn go_vendor_ledger(dir: &Path) -> bool {
    dir.join("modules.txt").is_file()
}

/// True when `dir` is a Composer vendored-dependency tree: it carries the
/// `composer/installed.json` ledger Composer writes at the `vendor/` root.
fn composer_vendor_ledger(dir: &Path) -> bool {
    dir.join("composer").join("installed.json").is_file()
}

/// Generic vendored-dependency subtree finder. Walks `project_root` for any
/// directory literally named `vendor` whose contents self-declare it via
/// `ledger` (the ecosystem's install inventory written at the `vendor/` root).
/// The matched `vendor` subtree is registered as a single prefix and not
/// descended into. Bounded depth-8 walk; dotted dirs and `node_modules` are
/// pruned. `already` are skipped to avoid duplicate entries.
fn vendor_ledger_prefixes(
    project_root: &Path,
    already: &[String],
    ledger: fn(&Path) -> bool,
) -> Vec<String> {
    const MAX_DEPTH: u32 = 8;
    let mut out = Vec::new();
    walk_for_vendor_ledger(project_root, project_root, 0, MAX_DEPTH, already, ledger, &mut out);
    out
}

/// Recursive helper for `vendor_ledger_prefixes`. Registers a `vendor`
/// directory whose contents satisfy `ledger`, then prunes it from the descent
/// (the prefix already covers everything below). Prunes dotted dirs and
/// `node_modules`.
#[allow(clippy::too_many_arguments)]
fn walk_for_vendor_ledger(
    project_root: &Path,
    dir: &Path,
    depth: u32,
    max_depth: u32,
    already: &[String],
    ledger: fn(&Path) -> bool,
    out: &mut Vec<String>,
) {
    if depth > max_depth {
        return;
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
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        if name == "vendor" && ledger(&path) {
            if let Ok(rel) = path.strip_prefix(project_root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                let rel = rel.trim_matches('/').to_string();
                if !rel.is_empty() && !already.iter().any(|p| p == &rel) {
                    // A self-declaring vendor tree is a leaf — its whole subtree
                    // is third-party, so don't descend into it.
                    out.push(rel);
                }
            }
            continue;
        }
        walk_for_vendor_ledger(project_root, &path, depth + 1, max_depth, already, ledger, out);
    }
}
