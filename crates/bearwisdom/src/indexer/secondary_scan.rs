// =============================================================================
// indexer/secondary_scan.rs — pull gitignored paths referenced by source imports
//
// `bearwisdom_profile::walk_files` is gitignore-aware: any directory listed
// in a `.gitignore` along the path gets skipped. That's correct for VCS but
// wrong for code intelligence when project source explicitly imports from a
// gitignored path. The canonical case is generated client code:
//
//   // hoppscotch-backend/src/some.service.ts
//   import { Prisma } from 'src/generated/prisma/client';
//
// where `src/generated/` is in `packages/hoppscotch-backend/.gitignore`. The
// import is unresolvable because the target dir was excluded from the index.
//
// This module runs a SECOND pass after the gitignore-aware walk: for every
// EcmaScript-family source file, scan its module specifiers, resolve each
// to a filesystem path under the project root, and add any existing files
// that the primary walk skipped. Hard-exclude directories (`node_modules`,
// `target`, `.git`, ...) are still ignored — those belong to the externals
// walker, not the source index.
//
// Source imports are an authoritative signal of intent. If the project's
// own code imports from a path, we want it indexed regardless of gitignore.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::walker::WalkedFile;

/// Scan every EcmaScript-family file in `primary` for module specifiers,
/// resolve them against the filesystem under `project_root`, and return any
/// files that aren't already in `primary` but exist on disk and look like
/// project source.
///
/// Returns an additive set — caller merges with the primary list.
pub fn pull_gitignored_imports(
    project_root: &Path,
    primary: &[WalkedFile],
) -> Vec<WalkedFile> {
    let walked_abs: HashSet<PathBuf> = primary
        .iter()
        .map(|f| f.absolute_path.clone())
        .collect();

    let project_root_canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());

    let mut found: HashSet<PathBuf> = HashSet::new();

    for file in primary {
        if !is_ecmascript_family(file.language) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&file.absolute_path) else {
            continue;
        };
        for spec in scan_import_specifiers(&content) {
            // Skip bare specifiers (`react`, `@scope/pkg`) — those go through
            // the externals walker. We're only interested in project-relative
            // paths that may have hit a gitignored directory.
            if !is_path_specifier(&spec) {
                continue;
            }
            for resolved in resolve_to_existing_files(
                &file.absolute_path,
                &project_root_canonical,
                &spec,
            ) {
                if walked_abs.contains(&resolved) || found.contains(&resolved) {
                    continue;
                }
                if !is_under_project_root(&resolved, &project_root_canonical) {
                    continue;
                }
                if has_hard_excluded_component(&resolved) {
                    continue;
                }
                found.insert(resolved);
            }
        }
    }

    found
        .into_iter()
        .filter_map(|path| {
            let language_id = crate::walker::detect_language(&path)?;
            let relative_path = path
                .strip_prefix(&project_root_canonical)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            Some(WalkedFile {
                relative_path,
                absolute_path: path,
                language: language_id,
            })
        })
        .collect()
}

fn is_ecmascript_family(lang: &str) -> bool {
    matches!(lang, "typescript" | "tsx" | "javascript" | "jsx" | "vue" | "svelte" | "astro" | "mdx")
}

/// Looks like a filesystem path (not a bare specifier).
///
/// Bare specifiers (`react`, `@tanstack/react-query`, `lodash/get`) start
/// with a letter or `@`, never a `.` or `/`. Path specifiers start with
/// `./`, `../`, `/`, or look like a project-relative path
/// (`src/foo`, `apps/web/x`). We treat anything containing a `/` AND not
/// starting with a letter-then-non-slash as a path candidate; bare-package
/// specifiers like `lodash/get` (letter + slash, where the first segment is
/// a real npm package) get filtered out by the on-disk check downstream
/// (the path won't exist under project root).
fn is_path_specifier(spec: &str) -> bool {
    if spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('/') {
        return true;
    }
    // Project-relative-shaped specifier (`src/foo`, `apps/web/x`,
    // `packages/.../bar`). Cheap pre-filter; the real check is whether
    // it resolves to an existing file under project root.
    if spec.contains('/') && !spec.starts_with('@') {
        return true;
    }
    false
}

/// Try every conventional resolution of `spec` against `from_file`'s dir
/// AND `project_root`. Returns every path that exists on disk AND is a
/// regular file (or an `index` file inside a directory).
fn resolve_to_existing_files(
    from_file: &Path,
    project_root: &Path,
    spec: &str,
) -> Vec<PathBuf> {
    let bases: Vec<PathBuf> = if spec.starts_with("./") || spec.starts_with("../") {
        // File-relative.
        vec![from_file.parent().unwrap_or(Path::new(".")).to_path_buf()]
    } else if spec.starts_with('/') {
        // Treat absolute as project-relative (Next.js convention).
        vec![project_root.to_path_buf()]
    } else {
        // Bare-shaped: try project root only.
        vec![project_root.to_path_buf()]
    };

    let cleaned = spec.trim_start_matches('/');
    let exts = [".ts", ".tsx", ".d.ts", ".js", ".jsx", ".mjs", ".cjs", ".vue", ".svelte", ".astro", ".mdx"];
    let mut out = Vec::new();
    for base in bases {
        let candidate = base.join(cleaned);

        // 1. Direct file as-is.
        if candidate.is_file() {
            out.push(candidate.clone());
        }
        // 2. With each extension appended (`./foo` → `./foo.ts`).
        for ext in exts {
            let with_ext = base.join(format!("{cleaned}{ext}"));
            if with_ext.is_file() {
                out.push(with_ext);
            }
        }
        // 3. Directory with `index.<ext>` inside (`./foo` → `./foo/index.ts`).
        if candidate.is_dir() {
            for ext in exts {
                let idx = candidate.join(format!("index{ext}"));
                if idx.is_file() {
                    out.push(idx);
                }
            }
        }
    }
    out
}

fn is_under_project_root(path: &Path, project_root: &Path) -> bool {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical.starts_with(project_root)
}

/// Hard-exclude directories that always belong to the externals walker or
/// build-output and never to the project source index — even when imported.
/// Skipping these here prevents the secondary pass from accidentally
/// pulling library source into the internal index.
fn has_hard_excluded_component(path: &Path) -> bool {
    const HARD_EXCLUDES: &[&str] = &[
        "node_modules", "target", ".git", ".next", ".nuxt", ".svelte-kit",
        ".turbo", ".vercel", ".cache", "__pycache__", "venv", ".venv",
    ];
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| HARD_EXCLUDES.contains(&s))
    })
}

/// Extract every import / require / dynamic-import / `from` specifier from
/// `source`. Single regex pass; tolerant of comment placement, line breaks
/// inside import groups, and arbitrary surrounding whitespace.
///
/// Why regex over tree-sitter: this pass runs over every EcmaScript file in
/// the project up-front. tree-sitter parsing each is ~10× slower than a
/// regex pass and we don't need a full AST — just every quoted module
/// specifier appearing after an import / from / require / import-call
/// keyword.
fn scan_import_specifiers(source: &str) -> Vec<String> {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r#"(?x)
            (?:
                # `import 'spec'`, `import { ... } from 'spec'`,
                # `import x from 'spec'`, `export { ... } from 'spec'`,
                # `export * from 'spec'`.
                \b (?: from | import ) \s+ ['"] ([^'"]+) ['"]
              | # `require('spec')` / `require( "spec" )`
                \b require \s* \( \s* ['"] ([^'"]+) ['"] \s* \)
              | # `import('spec')` (dynamic / type-only).
                \b import \s* \( \s* ['"] ([^'"]+) ['"] \s* \)
            )
            "#,
        )
        .expect("import-specifier regex")
    });
    let mut out = Vec::new();
    for cap in re.captures_iter(source) {
        if let Some(m) = cap
            .get(1)
            .or_else(|| cap.get(2))
            .or_else(|| cap.get(3))
        {
            out.push(m.as_str().to_string());
        }
    }
    out
}

#[cfg(test)]
#[path = "secondary_scan_tests.rs"]
mod tests;
