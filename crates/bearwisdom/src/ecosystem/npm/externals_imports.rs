// =============================================================================
// ecosystem/npm/externals_imports.rs — user-import and re-export scanning
//
// The user-import gate (collect_ts_user_imports) reduces the declared-dep
// list to the bare specifiers the project's source actually imports.
// extract_bare_reexport_specifiers + collect_bare_reexports_recursive walk
// the relative re-export chain inside a package to surface cross-package
// re-exports the entry file doesn't expose directly.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::walk::{
    extract_relative_reexports, resolve_relative_ts_path, REEXPORT_MAX_DEPTH,
};
use super::{is_valid_npm_module_path, npm_package_name_from_spec};

/// Walk the relative re-export chain starting at `entry`, collecting every
/// bare (cross-package) re-export specifier reachable via relative `./x` /
/// `../x` chains. Bounded by `REEXPORT_MAX_DEPTH` and a visited set so
/// cyclic re-exports (rare but seen in @types) don't loop.
///
/// Necessary because most npm packages keep cross-package re-exports out of
/// their entry file:
///   `dist/index.d.ts`     export * from './ts-estree'
///   `dist/ts-estree.d.ts` export { TSESTree } from '@typescript-eslint/types'
/// — scanning only the entry misses `@typescript-eslint/types`, leaving its
/// symbols absent from the index. Walking the relative chain catches them.
pub(crate) fn collect_bare_reexports_recursive(entry: &Path) -> Vec<String> {
    let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut stack: Vec<(PathBuf, u32)> = vec![(entry.to_path_buf(), 0)];
    while let Some((file, depth)) = stack.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        if depth > REEXPORT_MAX_DEPTH {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        for spec in extract_bare_reexport_specifiers(&src) {
            out.insert(spec);
        }
        for rel in extract_relative_reexports(&src) {
            if let Some(next) = resolve_relative_ts_path(&file, &rel) {
                stack.push((next, depth + 1));
            }
        }
    }
    out.into_iter().collect()
}

/// Scan a source file for `export ... from '<spec>'` / `import ... from '<spec>'`
/// statements where `spec` is a bare (non-relative) package specifier. Returns
/// the specifier's package name (e.g. `@vitest/expect`, `react`, `lodash`).
/// Relative specifiers are skipped — they stay within the current package and
/// are handled by `expand_reexports_into`.
pub(crate) fn extract_bare_reexport_specifiers(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if !(t.starts_with("export") || t.starts_with("import")) {
            continue;
        }
        let Some(ix) = t.find(" from ") else { continue };
        let rest = t[ix + 6..].trim_start();
        let Some(quote) = rest.chars().next() else {
            continue;
        };
        if quote != '\'' && quote != '"' {
            continue;
        }
        let inner = &rest[1..];
        let Some(end) = inner.find(quote) else {
            continue;
        };
        let spec = &inner[..end];
        if spec.starts_with("./") || spec.starts_with("../") {
            continue;
        }
        // Extract the package name: either `@scope/pkg` or `pkg` (first path segment).
        let pkg = if spec.starts_with('@') {
            spec.splitn(3, '/').take(2).collect::<Vec<_>>().join("/")
        } else {
            spec.split('/').next().unwrap_or(spec).to_string()
        };
        if !pkg.is_empty() {
            out.push(pkg);
        }
    }
    out
}

/// Collect every bare-specifier package the project's user source actually
/// imports. Used by `discover_ts_externals` to gate the declared-dep list
/// down to packages the application reaches.
///
/// Without this gate every dep in `package.json` becomes a dep root and gets
/// header-scanned by `build_npm_symbol_index`, even ones the user never
/// touches. Real projects routinely declare 100+ deps but import 30–50 —
/// scanning the unused ones is the dominant cost of npm externals indexing
/// (material-ui ships ~3 K declaration files; lodash, rxjs, three.js are
/// similar). User-import gating cuts the work by 50–70 % on typical
/// front-end checkouts.
///
/// `import_test_files` controls whether we walk `__tests__` / `*.spec.*`
/// trees. Production code usually doesn't import test fixtures, so the
/// scan skips test trees by default.
pub(crate) fn collect_ts_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut imports = std::collections::HashSet::new();
    scan_ts_user_imports_recursive(project_root, &mut imports, 0);
    imports
}

pub(crate) fn scan_ts_user_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Project test dirs are NOT pruned here: a dependency imported
                // only from test files (an assertion / matcher / DOM-query
                // library that no production file names) must still pass the
                // user-import gate so the transitive walker reaches it and the
                // chain walker can resolve its members. Build-output and
                // dependency-cache dirs stay pruned — they carry no
                // user-authored imports.
                if matches!(
                    name,
                    "node_modules"
                        | "target"
                        | "build"
                        | "out"
                        | "dist"
                        | ".next"
                        | ".nuxt"
                        | ".astro"
                        | ".svelte-kit"
                        | ".vite"
                        | ".turbo"
                        | ".cache"
                        | "coverage"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            scan_ts_user_imports_recursive(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !is_user_source_file(name) {
                continue;
            }
            // Skip declaration files — they're toolchain-emitted and may
            // re-export packages the user doesn't actually consume.
            if name.ends_with(".d.ts") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            extract_user_imports_from_source(&content, out);
        }
    }
}

/// File extensions that may contain user-authored TS/JS imports.
pub(crate) fn is_user_source_file(name: &str) -> bool {
    name.ends_with(".ts")
        || name.ends_with(".tsx")
        || name.ends_with(".mts")
        || name.ends_with(".cts")
        || name.ends_with(".js")
        || name.ends_with(".jsx")
        || name.ends_with(".mjs")
        || name.ends_with(".cjs")
        || name.ends_with(".vue")
        || name.ends_with(".svelte")
        || name.ends_with(".astro")
        || name.ends_with(".scss")
        || name.ends_with(".sass")
}

/// Tolerant scan for bare-specifier imports in user source. Recognized
/// shapes:
///   * `import ... from '<spec>'`
///   * `export ... from '<spec>'`
///   * `import '<spec>'`
///   * `require('<spec>')`
///   * `import('<spec>')` (dynamic)
///
/// Specifiers starting with `.`, `/`, or `node:` are skipped (relative,
/// absolute, builtin). Each retained specifier is reduced to its package
/// portion (`@scope/pkg/sub` → `@scope/pkg`, `pkg/dist/x` → `pkg`).
pub(crate) fn extract_user_imports_from_source(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    // Strategy: line-oriented scan picks up `from 'spec'` cheaply.
    // For `require('spec')` and `import('spec')` we additionally do a
    // single forward pass over the file content matching the `(` form,
    // since those calls appear inside expressions and aren't anchored
    // to a leading keyword.

    for line in content.lines() {
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        if t.starts_with("import ") || t.starts_with("export ") || t.starts_with("import\t") {
            if let Some(spec) = extract_quoted_after(t, " from ") {
                push_user_import(spec, out);
            } else if let Some(spec) = extract_bare_import_spec(t) {
                push_user_import(spec, out);
            }
        }
        // SCSS `@use`, `@import`, and `@forward` — line-oriented scan.
        // Sass built-in modules (`sass:*`) and relative paths are filtered
        // by `push_user_import` (starts with `.`) or the explicit sass: check.
        if t.starts_with("@use ") || t.starts_with("@import ") || t.starts_with("@forward ") {
            let after_keyword = t.splitn(2, ' ').nth(1).unwrap_or("").trim_start();
            if let Some(spec) = extract_first_quoted(after_keyword) {
                if !spec.starts_with("sass:") {
                    push_user_import(spec, out);
                }
            }
        }
    }

    // require('spec') and import('spec') — anywhere in the file.
    push_call_imports(content, "require(", out);
    push_call_imports(content, "import(", out);
}

/// `import 'pkg';` — no `from` clause. Returns the inner string of the
/// only quoted argument, or None.
pub(crate) fn extract_bare_import_spec(line: &str) -> Option<&str> {
    let after_import = line.strip_prefix("import ")?.trim_start();
    extract_first_quoted(after_import)
}

/// Find a quoted string occurring right after `marker` in `line`.
pub(crate) fn extract_quoted_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let ix = line.find(marker)?;
    let rest = line[ix + marker.len()..].trim_start();
    extract_first_quoted(rest)
}

/// Pick out the contents of the first single- or double-quoted string at the
/// start of `s`. Returns None if `s` doesn't begin with a quote.
pub(crate) fn extract_first_quoted(s: &str) -> Option<&str> {
    let quote = s.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let inner = &s[1..];
    let end = inner.find(quote)?;
    Some(&inner[..end])
}

/// Scan `content` for occurrences of `marker` (e.g. `require(`) followed by
/// a quoted bare specifier and push the package name into `out`.
pub(crate) fn push_call_imports(
    content: &str,
    marker: &str,
    out: &mut std::collections::HashSet<String>,
) {
    let mut cursor = 0usize;
    while let Some(rel) = content[cursor..].find(marker) {
        let absolute = cursor + rel + marker.len();
        cursor = absolute;
        let rest = &content[absolute..];
        let trimmed = rest.trim_start();
        if let Some(spec) = extract_first_quoted(trimmed) {
            push_user_import(spec, out);
        }
    }
}

/// Normalize a raw specifier and insert the package portion if it's bare.
pub(crate) fn push_user_import(spec: &str, out: &mut std::collections::HashSet<String>) {
    if spec.is_empty() {
        return;
    }
    if spec.starts_with('.') || spec.starts_with('/') {
        return;
    }
    if spec.starts_with("node:") {
        return;
    }
    // Windows drive letters (rare in source but possible in dynamic imports).
    if spec.len() >= 2 && spec.as_bytes()[1] == b':' {
        return;
    }
    let pkg = npm_package_name_from_spec(spec);
    if !is_valid_npm_module_path(pkg) {
        return;
    }
    out.insert(pkg.to_string());
}
