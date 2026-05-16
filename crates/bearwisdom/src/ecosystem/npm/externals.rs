// =============================================================================
// ecosystem/npm/externals.rs — TS externals pipeline (discovery, walk, scan, index)
//
// Everything that turns a project's `package.json` dependencies into
// `ExternalDepRoot` entries, walks the matching `node_modules/` for
// d.ts files, resolves package entry points, expands re-export chains, and
// builds the `(module, name) → file` symbol-location index that Stage 2
// demand resolution queries.
//
// Pipeline phases (top to bottom in this file):
//
//   1. Discovery — collect the project's import specifiers, locate each
//      package under one or more `node_modules/` candidates, build the
//      `ExternalDepRoot` list (`discover_ts_externals`).
//   2. Walking — read d.ts files from a dep root, resolve the primary entry
//      file from `package.json#types` / `main` / `exports`, expand
//      re-export chains (`walk_ts_external_root`).
//   3. Post-process — fix up parsed externals
//      (`ts_post_process_external`, `prefix_ts_external_symbols`,
//      `backfill_declare_global_symbols`).
//   4. Symbol index — header-scan every resolved entry / re-export-reachable
//      file and produce the cheap `SymbolLocationIndex` Stage 2 reads
//      (`build_npm_symbol_index`).
//
// All consumed by the `NpmEcosystem` / `ExternalSourceLocator` impls in
// `mod.rs`.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use crate::ecosystem::externals::{
    ts_package_from_virtual_path, ExternalDepRoot, MAX_WALK_DEPTH,
};
use crate::ecosystem::manifest::npm::NpmManifest;
use crate::ecosystem::manifest::ManifestReader;
use crate::ecosystem::SymbolLocationIndex;
use crate::walker::WalkedFile;

use super::{
    is_valid_npm_module_path, node_builtins, normalize_virtual_rel, npm_package_name_from_spec,
    package_declares_globals, package_ships_scss, scan_for_scss_bounded, LEGACY_ECOSYSTEM_TAG,
};
use super::walk::{
    extract_relative_reexports, is_test_or_story_file, resolve_package_entry_path,
    resolve_relative_ts_path, REEXPORT_MAX_DEPTH,
};

// ---------------------------------------------------------------------------
// Discovery — project-level
// ---------------------------------------------------------------------------

/// Discover all external TypeScript/JavaScript dependency roots for a project.
///
/// Strategy:
/// 1. Read package.json(s) via `NpmManifest` reader (already walks subdirs
///    and handles dependencies/devDependencies/peerDependencies).
/// 2. Locate node_modules via `BEARWISDOM_TS_NODE_MODULES` env → project-local
///    root → immediate subdirs.
/// 3. For each declared dep, resolve to `node_modules/{name}/` plus the
///    DefinitelyTyped `@types/` fallback for untyped packages.
/// 4. Skip Node builtins.
pub(crate) fn discover_ts_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let manifest = NpmManifest;
    let Some(data) = manifest.read(project_root) else { return Vec::new() };
    if data.dependencies.is_empty() { return Vec::new() }

    let node_modules_roots = find_node_modules(project_root);
    if node_modules_roots.is_empty() {
        debug!("No node_modules dirs discovered; skipping npm externals");
        return Vec::new();
    }

    // User-import gate: only emit dep roots for packages user code actually
    // imports. Material-ui ships ~3 K declaration files, lodash/rxjs/three.js
    // similar — header-scanning all of them when the user imports two
    // components is the dominant cost of npm externals indexing. A textual
    // scan over user source picks up `from 'pkg'`, `require('pkg')`, and
    // `import('pkg')` and reduces each to its package portion.
    //
    // Test-runner globals (`describe`, `it`, `expect`) are named bare in
    // user source without an `import`, so any dep whose entry .d.ts
    // declares globals (probe via `package_declares_globals`) is kept
    // regardless of whether the user wrote `import { describe } from
    // 'vitest'`. The companion `@types/<pkg>` package follows automatically
    // when the runtime package matches a user import.
    let user_imports = collect_ts_user_imports(project_root);
    debug!(
        "User-import gate: {} bare specifiers found in user source",
        user_imports.len()
    );

    debug!(
        "Probing {} node_modules root(s) for {} declared deps",
        node_modules_roots.len(),
        data.dependencies.len()
    );

    let builtins = node_builtins();
    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // When the project has SCSS source, retain any dep that ships `.scss`
    // files even if the user never writes `@use 'dep-name'`. SCSS test
    // frameworks (e.g. sass-true) and mixin libraries are loaded by the
    // build tool or test runner — not via an explicit SCSS `@use` in user
    // source — so the user-import gate would otherwise silently discard
    // them, leaving their mixins unresolvable.
    let project_has_scss = scan_for_scss_bounded(project_root, 0);

    for dep in &data.dependencies {
        if builtins.contains(dep.as_str()) { continue }
        if !is_valid_npm_module_path(dep) {
            debug!("npm: skipping invalid dep name `{dep}` from package.json");
            continue;
        }
        // Apply the user-import gate. A declared dep is kept iff:
        //   (a) the user imports it directly OR via its companion @types
        //       package, OR
        //   (b) its entry .d.ts declares globals (declare-global probe;
        //       see `package_declares_globals`), OR
        //   (c) the project uses SCSS and the dep ships `.scss` source
        //       (SCSS test frameworks + mixin libraries are runner-injected,
        //       not imported in user source), OR
        //   (d) the project has no scannable source (manifest-only
        //       checkouts, generators) — fall back to "keep all".
        if !user_imports.is_empty() {
            let companion = dep.strip_prefix("@types/");
            let user_imports_dep = user_imports.contains(dep);
            let user_imports_companion = companion
                .and_then(|c| {
                    // `@types/foo` is consumed when user imports `foo`.
                    // `@types/scope__pkg` is consumed when user imports `@scope/pkg`.
                    if let Some((scope, name)) = c.split_once("__") {
                        Some(format!("@{scope}/{name}"))
                    } else {
                        Some(c.to_string())
                    }
                })
                .map(|expanded| user_imports.contains(&expanded))
                .unwrap_or(false);
            let any_install_declares_globals = node_modules_roots.iter().any(|nm| {
                let primary = nm.join(dep);
                if primary.is_dir() && package_declares_globals(&primary) { return true; }
                if !dep.starts_with("@types/") {
                    if !dep.starts_with('@') {
                        let types_dir = nm.join("@types").join(dep);
                        if types_dir.is_dir() && package_declares_globals(&types_dir) { return true; }
                    } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                        let types_dir = nm.join("@types").join(&escaped);
                        if types_dir.is_dir() && package_declares_globals(&types_dir) { return true; }
                    }
                }
                false
            });
            let any_install_ships_scss = project_has_scss && node_modules_roots.iter().any(|nm| {
                let primary = nm.join(dep);
                primary.is_dir() && package_ships_scss(&primary)
            });
            if !user_imports_dep && !user_imports_companion
                && !any_install_declares_globals && !any_install_ships_scss
            {
                continue;
            }
        }

        // `@types/foo` deps are normally picked up as companions of `foo`
        // below, but for packages whose runtime ships pre-compiled JS
        // without inline types (jasmine, mocha, test runners, ambient-only
        // modules) the user declares `@types/foo` directly without a
        // matching `foo`. Those direct declarations must still get a
        // dep root so their `declare global { ... }` contents register
        // `describe`, `it`, `expect` etc. as global symbols.
        let is_types_only = dep.starts_with("@types/");

        // Each candidate is the directory plus the canonical npm
        // module_path that should label its content. The declaring `dep`
        // is the right label only for the runtime install — the
        // `@types/<name>` fallback directory is DefinitelyTyped content
        // and must keep its `@types/` prefix regardless of which dep led
        // us to it. Otherwise iteration order over `declared` (a
        // hash-randomised `HashSet`) decides whether `node_modules/@types/jest`
        // is labelled `jest` or `@types/jest`, which flips the heuristic
        // filter's `@types/`-substring classification on/off across runs.
        // The TS resolver's `ts_import_definitely_typed` retry handles
        // the `import from 'jest'` lookup against the `@types/jest` qname
        // independently, so this label change does not break consumers.
        let mut pkg_roots: Vec<(PathBuf, String)> = Vec::new();
        for nm_root in &node_modules_roots {
            let primary = nm_root.join(dep);
            if primary.is_dir() { pkg_roots.push((primary, dep.clone())) }
            if !is_types_only {
                if !dep.starts_with('@') {
                    let types_dir = nm_root.join("@types").join(dep);
                    if types_dir.is_dir() {
                        pkg_roots.push((types_dir, format!("@types/{dep}")));
                    }
                } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                    let types_dir = nm_root.join("@types").join(&escaped);
                    if types_dir.is_dir() {
                        pkg_roots.push((types_dir, format!("@types/{escaped}")));
                    }
                }
            }
        }

        for (pkg_dir, module_path) in pkg_roots {
            if seen.insert(pkg_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path,
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    // Transitive dep expansion: for each declared dep, follow the
    // cross-package re-exports in its type-entry `.d.ts`. Pattern seen
    // with vitest: its entry file has
    //   export { X } from '@vitest/expect'
    //   export { Y } from '@vitest/runner'
    // but `@vitest/expect` / `@vitest/runner` aren't in vitest's
    // package.json — they're installed via the lockfile. Without this
    // step, demand-driven resolution never finds the interfaces that
    // define matcher chains.
    //
    // Iterate to a fixed point so multi-hop re-export chains are walked
    // completely. Playwright is the canonical case: `@playwright/test/index.d.ts`
    // re-exports `playwright/test`, which re-exports `./types/test`, which
    // re-exports `playwright-core` — three packages, three hops. A single
    // pass would stop at `playwright` and miss `playwright-core` (where
    // `Page.getByRole`, `Locator.click` and the rest of the API live).
    //
    // Bounded by `MAX_TRANSITIVE_PASSES` so a pathological re-export graph
    // can't loop indefinitely; each pass only walks the entries of newly-
    // added roots, so the cost stays O(deps × avg_entry_size), not the
    // full dependency tree per pass.
    const MAX_TRANSITIVE_PASSES: u32 = 5;
    let builtins_set: std::collections::HashSet<&str> = builtins.iter().copied().collect();
    let mut next_pass_start: usize = 0;
    for _pass in 0..MAX_TRANSITIVE_PASSES {
        // Snapshot of the existing set at the START of this pass — used to
        // skip specs we already have a root for.
        let existing: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        // Each transitive spec is paired with its origin dep's own local
        // `node_modules/` — pnpm stores transitive packages there as siblings,
        // not at the workspace's top-level node_modules. Without this, packages
        // re-exported from a dep but not declared in the consumer's
        // package.json (e.g. `@typescript-eslint/types` re-exported through
        // `@typescript-eslint/utils`) stay invisible.
        let mut transitive_specs: std::collections::HashSet<(String, PathBuf)> =
            std::collections::HashSet::new();
        // Only walk the roots added in the previous pass (or all roots on
        // pass 0). On a fixed graph this converges in 1–4 passes.
        let scan_range = next_pass_start..roots.len();
        if scan_range.is_empty() { break }
        for idx in scan_range.clone() {
            let r = &roots[idx];
            let entry = match resolve_package_entry_path(r) {
                Some(e) => e,
                None => continue,
            };
            let local_nm = dep_local_node_modules(&r.root).unwrap_or_default();
            for spec in collect_bare_reexports_recursive(&entry) {
                if !existing.contains(&spec) && !builtins_set.contains(spec.as_str()) {
                    transitive_specs.insert((spec, local_nm.clone()));
                }
            }
        }

        if transitive_specs.is_empty() { break }
        next_pass_start = roots.len();

    for (spec, parent_local_nm) in transitive_specs {
        // Deep re-export specs like `export * from 'playwright/test'` point
        // at a submodule of a transitive package — the package name is the
        // prefix portion, the rest is an in-package path. Reduce the spec to
        // its package portion: `playwright/test` → `playwright`,
        // `@types/node/fs/promises` → `@types/node`, `@vitest/expect` →
        // `@vitest/expect` (no change). Then walk that whole package; any
        // re-export the user actually relies on is reachable from the
        // package's regular entry points + the demand-driven BFS that picks
        // up sibling files. Without this, every cross-package deep
        // re-export (Playwright → playwright-core, Mongoose's submodule
        // exports, RxJS's `rxjs/operators`) silently fails to walk the
        // target package and the chain walker can't find the methods.
        let package_spec = npm_package_name_from_spec(&spec);
        if !is_valid_npm_module_path(package_spec) {
            debug!("npm: skipping invalid transitive spec `{spec}`");
            continue;
        }
        // Try the standard workspace node_modules roots first (npm/yarn
        // hoist transitives there). Fall back to the parent dep's own
        // `node_modules/` (pnpm stores them there).
        let mut probe_roots: Vec<&Path> =
            node_modules_roots.iter().map(|p| p.as_path()).collect();
        if !parent_local_nm.as_os_str().is_empty() {
            probe_roots.push(parent_local_nm.as_path());
        }
        for nm_root in probe_roots {
            let candidate = nm_root.join(package_spec);
            if !candidate.is_dir() { continue }
            if !seen.insert(candidate.clone()) { continue }
            roots.push(ExternalDepRoot {
                module_path: package_spec.to_string(),
                version: String::from("unknown"),
                root: candidate,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
            break; // one canonical install per spec is enough
        }
    }
    }

    roots
}

/// Compute a dep's own `node_modules/` directory — the place where pnpm
/// stores its transitive dependencies as siblings. For a package at
/// `<store>/node_modules/<scope>/<name>` returns `<store>/node_modules`,
/// for `<store>/node_modules/<name>` returns `<store>/node_modules`. None
/// when the directory layout doesn't match either shape.
///
/// Resolves through symlinks first so pnpm's symlink layout
/// (`pkg/node_modules/<scope>/<dep>` → `<store>/node_modules/<scope>/<dep>`)
/// produces the real on-disk store path, where the dep's own deps live as
/// siblings. Without canonicalisation the parent walks land on the symlink
/// container, which only holds packages declared in the consumer's own
/// `package.json`, missing every transitive.
pub(crate) fn dep_local_node_modules(dep_root: &Path) -> Option<PathBuf> {
    let real_root = std::fs::canonicalize(dep_root).ok().unwrap_or_else(|| dep_root.to_path_buf());
    let parent = real_root.parent()?;
    let parent_name = parent.file_name()?.to_str()?;
    if parent_name.starts_with('@') {
        parent.parent().map(|p| p.to_path_buf())
    } else {
        Some(parent.to_path_buf())
    }
}

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
        if !seen.insert(file.clone()) { continue }
        if depth > REEXPORT_MAX_DEPTH { continue }
        let Ok(src) = std::fs::read_to_string(&file) else { continue };
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
        if !(t.starts_with("export") || t.starts_with("import")) { continue }
        let Some(ix) = t.find(" from ") else { continue };
        let rest = t[ix + 6..].trim_start();
        let Some(quote) = rest.chars().next() else { continue };
        if quote != '\'' && quote != '"' { continue }
        let inner = &rest[1..];
        let Some(end) = inner.find(quote) else { continue };
        let spec = &inner[..end];
        if spec.starts_with("./") || spec.starts_with("../") { continue }
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
/// scan skips test trees by default; the `demand_pre_pull_test_globals`
/// path covers the symbols those files would have brought in via
/// declare-global blocks in test-runner packages.
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
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "node_modules" | "target" | "build" | "out" | "dist"
                        | ".next" | ".nuxt" | ".astro" | ".svelte-kit"
                        | ".vite" | ".turbo" | ".cache" | "coverage"
                        | "__tests__" | "__mocks__" | "tests" | "test"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            scan_ts_user_imports_recursive(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !is_user_source_file(name) { continue }
            // Skip declaration files — they're toolchain-emitted and may
            // re-export packages the user doesn't actually consume.
            if name.ends_with(".d.ts") { continue }
            // Skip per-file test/story names — same rationale as the dir
            // skip above.
            if is_test_or_story_file(name) { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
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
        if t.starts_with("//") { continue }
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
            let after_keyword = t
                .splitn(2, ' ')
                .nth(1)
                .unwrap_or("")
                .trim_start();
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
    if quote != '\'' && quote != '"' { return None }
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
    if spec.is_empty() { return }
    if spec.starts_with('.') || spec.starts_with('/') { return }
    if spec.starts_with("node:") { return }
    // Windows drive letters (rare in source but possible in dynamic imports).
    if spec.len() >= 2 && spec.as_bytes()[1] == b':' { return }
    let pkg = npm_package_name_from_spec(spec);
    if !is_valid_npm_module_path(pkg) { return }
    out.insert(pkg.to_string());
}

/// DefinitelyTyped publishes types for scoped packages at
/// `@types/{scope}__{name}` because npm disallows nested `@` inside a scope
/// path. Returns None for non-scoped names.
pub(crate) fn definitely_typed_scoped_name(dep: &str) -> Option<String> {
    let rest = dep.strip_prefix('@')?;
    let (scope, name) = rest.split_once('/')?;
    if scope.is_empty() || name.is_empty() { return None }
    Some(format!("{scope}__{name}"))
}

// ---------------------------------------------------------------------------
// Discovery — per-package (monorepo M3)
// ---------------------------------------------------------------------------

/// Per-package variant. Reads the single package's `package.json` AND the
/// workspace root's, then merges the dep sets — covers the standard
/// npm/yarn monorepo pattern where root-level devDependencies hold shared
/// test tooling (chai, vitest, jest) that no individual sub-package
/// redeclares. Searches `{package}/node_modules` plus every ancestor up to
/// `workspace_root` (inclusive) for hoisted deps.
pub(crate) fn discover_ts_externals_scoped(
    workspace_root: &Path,
    package_abs_path: &Path,
) -> Vec<ExternalDepRoot> {
    // Primary: the package's own package.json (classic monorepo layout —
    // one package.json per package dir).
    let mut declared = read_single_package_json_deps(package_abs_path).unwrap_or_default();

    // Fallback: polyglot repos where a .NET / Rust / Go package owns a
    // nested TypeScript sub-app with its own package.json
    // (`src/Web/ClientApp/package.json` in Clean-Architecture layouts,
    // `web/frontend/package.json` in Go services, etc.). Walk the
    // package subtree for any package.json files and merge their deps.
    // Skip `node_modules/` during the walk so we don't pick up
    // third-party package.json files.
    declared.extend(read_nested_package_json_deps(package_abs_path));

    if package_abs_path != workspace_root {
        if let Some(root_deps) = read_single_package_json_deps(workspace_root) {
            declared.extend(root_deps);
        }
    }

    if declared.is_empty() { return Vec::new() }

    let node_modules_roots = find_node_modules_with_ancestors(package_abs_path, workspace_root);
    if node_modules_roots.is_empty() {
        debug!(
            "No node_modules dirs discovered for package at {}; skipping npm externals",
            package_abs_path.display()
        );
        return Vec::new();
    }

    // Same user-import gate as the project-level path. Walks the package's
    // own source tree (not the entire workspace) so the import set reflects
    // what THIS package consumes — a sibling package's React import
    // doesn't make React relevant here.
    let user_imports = collect_ts_user_imports(package_abs_path);

    // SCSS dep survival: when workspace root (or the package itself) has SCSS
    // source, any dep that ships .scss files is kept. Mirrors the logic in
    // discover_ts_externals for the per-package path.
    let project_has_scss = scan_for_scss_bounded(workspace_root, 0)
        || (package_abs_path != workspace_root && scan_for_scss_bounded(package_abs_path, 0));

    let builtins = node_builtins();
    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for dep in &declared {
        if builtins.contains(dep.as_str()) { continue }
        if !is_valid_npm_module_path(dep) {
            debug!("npm: skipping invalid scoped dep name `{dep}` from package.json");
            continue;
        }
        // See discover_ts_externals — same gate, content-based fallback for
        // packages that contribute globals via `declare global` / top-level
        // `declare namespace`.
        if !user_imports.is_empty() {
            let companion = dep.strip_prefix("@types/");
            let user_imports_dep = user_imports.contains(dep);
            let user_imports_companion = companion
                .map(|c| {
                    if let Some((scope, name)) = c.split_once("__") {
                        format!("@{scope}/{name}")
                    } else {
                        c.to_string()
                    }
                })
                .map(|expanded| user_imports.contains(&expanded))
                .unwrap_or(false);
            let any_install_declares_globals = node_modules_roots.iter().any(|nm| {
                let primary = nm.join(dep);
                if primary.is_dir() && package_declares_globals(&primary) { return true; }
                if !dep.starts_with("@types/") {
                    if !dep.starts_with('@') {
                        let types_dir = nm.join("@types").join(dep);
                        if types_dir.is_dir() && package_declares_globals(&types_dir) { return true; }
                    } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                        let types_dir = nm.join("@types").join(&escaped);
                        if types_dir.is_dir() && package_declares_globals(&types_dir) { return true; }
                    }
                }
                false
            });
            let any_install_ships_scss = project_has_scss && node_modules_roots.iter().any(|nm| {
                let primary = nm.join(dep);
                primary.is_dir() && package_ships_scss(&primary)
            });
            if !user_imports_dep && !user_imports_companion
                && !any_install_declares_globals && !any_install_ships_scss
            {
                continue;
            }
        }
        // See `discover_ts_externals` — direct `@types/foo` declarations
        // must not be silently dropped; they may be the sole type source
        // for an ambient-only package (jasmine, mocha, etc.).
        let is_types_only = dep.starts_with("@types/");

        // Carry the canonical module_path alongside each candidate
        // directory. See the matching block in `discover_ts_externals`
        // for the rationale — `node_modules/@types/X` always labels
        // as `@types/X` regardless of which declared dep led us here.
        let mut pkg_roots: Vec<(PathBuf, String)> = Vec::new();
        for nm_root in &node_modules_roots {
            let primary = nm_root.join(dep);
            if primary.is_dir() { pkg_roots.push((primary, dep.clone())) }
            if !is_types_only {
                if !dep.starts_with('@') {
                    let types_dir = nm_root.join("@types").join(dep);
                    if types_dir.is_dir() {
                        pkg_roots.push((types_dir, format!("@types/{dep}")));
                    }
                } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                    let types_dir = nm_root.join("@types").join(&escaped);
                    if types_dir.is_dir() {
                        pkg_roots.push((types_dir, format!("@types/{escaped}")));
                    }
                }
            }
        }
        for (pkg_dir, module_path) in pkg_roots {
            if seen.insert(pkg_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path,
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    // Transitive re-export expansion — mirror of the logic in
    // `discover_ts_externals`, but iterated to a fixed point so multi-hop
    // re-export chains converge. The canonical case is Playwright:
    // `@playwright/test/index.d.ts` → `playwright/test` → (via relative
    // chain) `./types/test` → `playwright-core`. A single pass would stop
    // at `playwright` and miss `playwright-core`, where `Page.getByRole`,
    // `Locator.click` and the rest of the API surface lives.
    const MAX_TRANSITIVE_PASSES: u32 = 5;
    let builtins_set: std::collections::HashSet<&str> = builtins.iter().copied().collect();
    let mut next_pass_start: usize = 0;
    for _pass in 0..MAX_TRANSITIVE_PASSES {
        let existing: std::collections::HashSet<String> =
            roots.iter().map(|r| r.module_path.clone()).collect();
        let mut transitive_specs: std::collections::HashSet<(String, PathBuf)> =
            std::collections::HashSet::new();
        let scan_range = next_pass_start..roots.len();
        if scan_range.is_empty() { break }
        for idx in scan_range {
            let r = &roots[idx];
            let entry = match resolve_package_entry_path(r) {
                Some(e) => e,
                None => continue,
            };
            let local_nm = dep_local_node_modules(&r.root).unwrap_or_default();
            for spec in collect_bare_reexports_recursive(&entry) {
                if !existing.contains(&spec) && !builtins_set.contains(spec.as_str()) {
                    transitive_specs.insert((spec, local_nm.clone()));
                }
            }
        }

        if transitive_specs.is_empty() { break }
        next_pass_start = roots.len();
        for (spec, parent_local_nm) in transitive_specs {
            // Reduce deep specs (`playwright/test`, `@types/node/fs`) to
            // their package portion before validating + walking. The
            // bare-spec extractor already does this for output, but
            // intermediate re-exports passed in via the lockfile / npm
            // packaging may contain raw deep specifiers — handle both.
            let package_spec = npm_package_name_from_spec(&spec);
            if !is_valid_npm_module_path(package_spec) {
                debug!("npm: skipping invalid scoped transitive spec `{spec}`");
                continue;
            }
            let mut probe_roots: Vec<&Path> =
                node_modules_roots.iter().map(|p| p.as_path()).collect();
            if !parent_local_nm.as_os_str().is_empty() {
                probe_roots.push(parent_local_nm.as_path());
            }
            for nm_root in probe_roots {
                let candidate = nm_root.join(package_spec);
                if !candidate.is_dir() { continue }
                if !seen.insert(candidate.clone()) { continue }
                roots.push(ExternalDepRoot {
                    module_path: package_spec.to_string(),
                    version: String::from("unknown"),
                    root: candidate,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                    requested_imports: Vec::new(),
                });
                break;
            }
        }
    }

    roots
}

pub(crate) fn read_single_package_json_deps(dir: &Path) -> Option<std::collections::HashSet<String>> {
    let manifest_path = dir.join("package.json");
    let content = std::fs::read_to_string(&manifest_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let obj = value.as_object()?;
    let mut deps = std::collections::HashSet::new();
    for field in &["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(map) = obj.get(*field).and_then(|v| v.as_object()) {
            for key in map.keys() {
                if key.starts_with('@') {
                    if let Some(scope) = key.split('/').next() {
                        deps.insert(scope.to_string());
                    }
                }
                deps.insert(key.clone());
            }
        }
    }
    Some(deps)
}

/// Walk `dir`'s subtree for `package.json` files (skipping `node_modules/`
/// so we don't pick up third-party manifests) and union the declared
/// dependencies. Used to discover deps from a TypeScript sub-app nested
/// inside a non-TS package — e.g. `src/Web/ClientApp/package.json` inside
/// a .NET `src/Web` project.
///
/// Bounded depth (6) keeps the walk cheap on real repos; in practice
/// the nested manifest is at most 2–3 levels below the package root.
pub(crate) fn read_nested_package_json_deps(dir: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    walk_for_package_json(dir, dir, &mut out, 0);
    out
}

pub(crate) fn walk_for_package_json(
    cur: &Path,
    root: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(cur) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let name = entry.file_name();
        let name_lossy = name.to_string_lossy();
        if file_type.is_dir() {
            // Skip canonical exclusion dirs so we don't recurse into
            // artifacts or vendor code.
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | ".turbo" | ".next"
                        | "bin" | "obj" | "coverage"
                )
            {
                continue;
            }
            walk_for_package_json(&entry.path(), root, out, depth + 1);
        } else if name == "package.json" {
            // Skip the exact same package.json the caller already read
            // (when cur == root).
            if entry.path().parent() == Some(root) {
                continue;
            }
            if let Some(deps) = read_single_package_json_deps(
                entry.path().parent().unwrap_or(cur),
            ) {
                out.extend(deps);
            }
        }
    }
}

pub(crate) fn find_node_modules_with_ancestors(start: &Path, workspace_root: &Path) -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        let mut out = Vec::new();
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue }
            if seg.is_dir() && !out.contains(&seg) { out.push(seg) }
        }
        if !out.is_empty() { return out }
    }

    let mut out: Vec<PathBuf> = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) { out.push(p) }
    };

    push_if_dir(start.join("node_modules"), &mut out);
    // Walk the subtree under `start` (bounded depth) for nested
    // node_modules dirs. Polyglot layouts bury their TypeScript
    // sub-apps 2–3 levels deep inside a non-TS package
    // (`src/Web/ClientApp/node_modules`, `web/admin/node_modules`, …)
    // and the old single-level scan missed them. Skips
    // `node_modules/` / build-artifact dirs so we don't recurse into
    // third-party trees.
    walk_for_nested_node_modules(start, &mut out, 0);

    let mut current = start.parent();
    while let Some(dir) = current {
        push_if_dir(dir.join("node_modules"), &mut out);
        if dir == workspace_root { break }
        current = dir.parent();
    }
    out
}

pub(crate) fn walk_for_nested_node_modules(cur: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(cur) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name_lossy = name.to_string_lossy();
        if name_lossy == "node_modules" {
            let path = entry.path();
            if !out.contains(&path) {
                out.push(path);
            }
            // Don't recurse into node_modules — we want the outermost
            // for each install pocket.
            continue;
        }
        if name_lossy.starts_with('.')
            || matches!(
                name_lossy.as_ref(),
                "target" | "dist" | "build" | ".turbo" | ".next" | "bin" | "obj" | "coverage"
            )
        {
            continue;
        }
        walk_for_nested_node_modules(&entry.path(), out, depth + 1);
    }
}

pub(crate) fn find_node_modules(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) { out.push(p) }
    };

    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue }
            push_if_dir(seg, &mut out);
        }
        if !out.is_empty() { return out }
    }

    // Walk the project tree looking for top-level `node_modules/` dirs at
    // ANY depth. Polyglot repos routinely bury their TypeScript side deep
    // inside a backend layout (`src/Web/ClientApp/`, `frontend/`,
    // `apps/web/`, `webclient/`, …) and the prior single-level scan missed
    // them, leaving every imported package unresolved.
    //
    // Each discovered `node_modules/` dir is recorded exactly once and we
    // do NOT descend into one to find nested ones (that path is reserved
    // for `BEARWISDOM_TS_WALK_NESTED` and npm package-level walking —
    // it's not how app-level dependency discovery works).
    //
    // CRITICAL: do NOT enable gitignore. Every JS/TS project's
    // `.gitignore` starts with `node_modules/`, which would hide the
    // very directory we're looking for. We keep the default filters
    // that ignore hidden dirs (`.git/`, `.turbo/`, `.next/`) and
    // common build-artifact trees so the walk stays cheap, but
    // gitignore matching is disabled explicitly.
    use ignore::WalkBuilder;
    let walker = WalkBuilder::new(project_root)
        .follow_links(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .hidden(true)
        .filter_entry(|entry| {
            // Prune artifact trees to keep the scan bounded without
            // needing gitignore rules.
            let name = entry.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "target" | "dist" | "build" | ".turbo" | ".next" | "bin" | "obj"
            )
        })
        .build();

    for entry in walker.flatten() {
        let p = entry.path();
        if !p.file_name().map(|n| n == "node_modules").unwrap_or(false) {
            continue;
        }
        if !p.is_dir() {
            continue;
        }
        // Skip nested node_modules — only count the outermost for any
        // given package-install pocket.
        let is_nested = p
            .ancestors()
            .skip(1)
            .any(|a| a.file_name().map(|n| n == "node_modules").unwrap_or(false));
        if is_nested {
            continue;
        }
        push_if_dir(p.to_path_buf(), &mut out);
    }

    out
}

