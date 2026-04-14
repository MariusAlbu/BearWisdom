// Go module cache discovery + walker

use super::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::indexer::manifest::go_mod::{find_go_mod, parse_go_mod, GoModDep};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Go module cache → `discover_go_externals` + `walk_external_root`.
pub struct GoExternalsLocator;

impl ExternalSourceLocator for GoExternalsLocator {
    fn ecosystem(&self) -> &'static str { "go" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_go_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_external_root(dep)
    }
}

/// Discover all external Go dependency roots for a project.
///
/// Strategy: parse `go.mod`, resolve each direct `require` entry to
/// `$GOMODCACHE/{escaped_module_path}@{version}`, and return the entries
/// whose directory actually exists on disk.
///
/// **Indirect deps are walked only when user code imports them.** A
/// lightweight string scan over `.go` files produces a set of imported
/// module paths; any `go.mod // indirect` entry whose module path (or
/// a prefix of it) appears in that set gets walked too. This catches
/// legitimate transitive exposure — e.g. when a direct dep re-exports
/// types from a transitive dep and user code references those types
/// directly — without the 10-20x symbol table explosion that walking
/// every indirect dep would cause in projects like Gitea (314 indirect
/// vs 14 direct). Indirect deps that user code never touches are still
/// skipped.
pub fn discover_go_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let Some(go_mod_path) = find_go_mod(project_root) else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&go_mod_path) else {
        return Vec::new();
    };
    let parsed = parse_go_mod(&content);

    let cache_root = match gomodcache_root() {
        Some(p) => p,
        None => {
            debug!("No GOMODCACHE / GOPATH detected; skipping Go externals");
            return Vec::new();
        }
    };

    // For indirect deps, only walk those that user code actually imports.
    // This catches genuine transitive type exposure (e.g., a direct dep
    // re-exports types from a transitive) without the 10-20x symbol table
    // explosion that walking every indirect dep causes in projects like
    // Gitea (314 indirect vs 14 direct). The user-imports set is built by
    // a lightweight string scan over .go files — no tree-sitter needed.
    let user_imports = collect_go_imports(project_root);

    let mut roots = Vec::new();
    for dep in &parsed.require_deps {
        if dep.indirect && !go_dep_is_imported(&dep.path, &user_imports) {
            continue;
        }
        if let Some(root) = resolve_go_dep_path(&cache_root, dep) {
            roots.push(ExternalDepRoot {
                module_path: dep.path.clone(),
                version: dep.version.clone(),
                root,
                ecosystem: "go",
                package_id: None,
            });
        } else {
            debug!(
                "Go module cache miss for {}@{} — not found under {}",
                dep.path,
                dep.version,
                cache_root.display()
            );
        }
    }
    roots
}

/// Scan `.go` files under `project_root` for import strings. Returns a
/// set of unique import paths. Skips vendor/, tests, and common build
/// output directories. This is a lightweight string search — importing
/// tree-sitter here would be overkill for what amounts to a regex.
fn collect_go_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut imports: std::collections::HashSet<String> = std::collections::HashSet::new();
    scan_go_imports_recursive(project_root, &mut imports, 0);
    imports
}

fn scan_go_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 10 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        ".git" | "vendor" | "node_modules" | "target"
                            | "build" | "dist" | "testdata"
                    ) {
                        continue;
                    }
                }
                scan_go_imports_recursive(&path, out, depth + 1);
            } else if ft.is_file() {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !name.ends_with(".go") || name.ends_with("_test.go") {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                extract_imports_from_go_source(&content, out);
            }
        }
    }
}

/// Parse an `import "..."` or block `import (\n"..."\n)` from Go source.
/// Intentionally loose — we'd rather include a false positive path string
/// from a comment than miss a real import and under-count transitives.
/// The downstream filter `go_dep_is_imported` uses prefix matching so a
/// stray false positive is inert unless it exactly matches a go.mod dep.
fn extract_imports_from_go_source(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    enum Mode {
        Top,
        InBlock,
    }
    let mut mode = Mode::Top;
    for line in content.lines() {
        let trimmed = line.trim();
        match mode {
            Mode::Top => {
                if trimmed.starts_with("import (") {
                    mode = Mode::InBlock;
                    continue;
                }
                if let Some(rest) = trimmed.strip_prefix("import ") {
                    let rest = rest.trim_start_matches('_').trim();
                    // Support aliased imports: `import foo "..."`
                    let quoted = rest
                        .rsplit_once('"')
                        .map(|(head, _)| head)
                        .and_then(|head| head.rsplit_once('"').map(|(_, s)| s));
                    if let Some(path) = quoted {
                        if !path.is_empty() {
                            out.insert(path.to_string());
                        }
                    }
                }
            }
            Mode::InBlock => {
                if trimmed == ")" {
                    mode = Mode::Top;
                    continue;
                }
                // Line might look like: `"fmt"`, `foo "github.com/x/y"`,
                // `_ "database/sql/driver"`. Extract the first quoted string.
                let bytes = trimmed.as_bytes();
                let first = bytes.iter().position(|&b| b == b'"');
                let Some(start) = first else { continue };
                let after = &trimmed[start + 1..];
                let Some(end_rel) = after.find('"') else { continue };
                let path = &after[..end_rel];
                if !path.is_empty() {
                    out.insert(path.to_string());
                }
            }
        }
    }
}

/// Does any user import path start with the given dep module path?
/// Go imports subpackages (`github.com/foo/bar/subpkg`) of declared
/// modules (`github.com/foo/bar`), so prefix matching is the right test.
fn go_dep_is_imported(
    dep_path: &str,
    user_imports: &std::collections::HashSet<String>,
) -> bool {
    if user_imports.contains(dep_path) {
        return true;
    }
    let prefix = format!("{dep_path}/");
    user_imports.iter().any(|imp| imp.starts_with(&prefix))
}

/// Resolve the on-disk location of `$GOMODCACHE`.
///
/// Order: `$GOMODCACHE` env → `$GOPATH/pkg/mod` → `$HOME/go/pkg/mod`
/// (or `$USERPROFILE\go\pkg\mod` on Windows).
pub fn gomodcache_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("GOMODCACHE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(gopath) = std::env::var_os("GOPATH") {
        // GOPATH may be a colon/semicolon-separated list; use the first entry.
        let first = PathBuf::from(gopath)
            .to_string_lossy()
            .split(|c| c == ':' || c == ';')
            .next()
            .map(PathBuf::from);
        if let Some(p) = first {
            let candidate = p.join("pkg").join("mod");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join("go").join("pkg").join("mod");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Build the cache directory path for one `require` entry.
///
/// Go applies case-escaping to module paths: every uppercase letter becomes
/// `!<lowercase>`. For example, `github.com/Microsoft/go-winio` lives at
/// `github.com/!microsoft/go-winio@v0.6.2`.
fn resolve_go_dep_path(cache_root: &Path, dep: &GoModDep) -> Option<PathBuf> {
    let escaped = escape_module_path(&dep.path);
    let dirname = format!("{}@{}", escaped, dep.version);
    // The escaped module path may contain `/` which maps to real subdirs.
    let candidate = cache_root.join(dirname.replace('/', std::path::MAIN_SEPARATOR_STR));
    if candidate.is_dir() {
        return Some(candidate);
    }
    // Fall back: split escaped path and join the final segment with the version.
    // e.g., github.com/foo/bar → github.com/foo/bar@v1.2.3
    let mut segments: Vec<&str> = escaped.split('/').collect();
    let last = segments.pop()?;
    let mut path = cache_root.to_path_buf();
    for seg in segments {
        path.push(seg);
    }
    path.push(format!("{last}@{}", dep.version));
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

/// Apply Go module path case escaping: uppercase → `!lowercase`.
fn escape_module_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 4);
    for ch in path.chars() {
        if ch.is_ascii_uppercase() {
            out.push('!');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Walk one external dependency root and emit `WalkedFile` entries for every
/// source file the indexer knows how to parse.
///
/// File filtering rules (Go-specific for now):
/// - Only `.go` files.
/// - Skip `*_test.go` — test files aren't part of the public API surface.
/// - Skip `internal/testdata/` and `vendor/` subtrees.
///
/// `relative_path` on the returned entries is given as
/// `ext:{module_path}@{version}/{sub_path}` so external and internal files
/// never collide in the files.path unique index.
pub fn walk_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_dir_bounded(dir, root, dep, out, 0);
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "vendor" | "testdata" | ".git" | "_examples") {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".go") {
                continue;
            }
            if name.ends_with("_test.go") {
                continue;
            }

            // Build the virtual path: ext:{module}@{version}/{sub_path}
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:{}@{}/{}", dep.module_path, dep.version, rel_sub);

            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "go",
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_preserves_lowercase_paths() {
        assert_eq!(
            escape_module_path("github.com/gin-gonic/gin"),
            "github.com/gin-gonic/gin"
        );
    }

    #[test]
    fn escape_handles_uppercase_segments() {
        assert_eq!(
            escape_module_path("github.com/Microsoft/go-winio"),
            "github.com/!microsoft/go-winio"
        );
        assert_eq!(
            escape_module_path("github.com/AlecAivazis/survey"),
            "github.com/!alec!aivazis/survey"
        );
    }

    #[test]
    fn discover_returns_empty_without_go_mod() {
        let tmp = std::env::temp_dir().join("bw-test-no-go-mod");
        let _ = std::fs::create_dir_all(&tmp);
        let result = discover_go_externals(&tmp);
        assert!(result.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
