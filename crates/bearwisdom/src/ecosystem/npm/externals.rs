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

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

/// Walk one npm external dep root and emit `WalkedFile` entries.
///
/// File filtering rules:
/// - Include `.ts`, `.tsx`, `.d.ts`, `.mts`, `.cts`, `.d.mts`, `.d.cts`.
/// - Skip `.js`/`.jsx`/`.mjs` (type info lives in sibling `.d.ts`).
/// - Skip nested `node_modules/` unless `BEARWISDOM_TS_WALK_NESTED=1`.
/// - Skip test/story/example/fixture dirs and files.
///
/// Virtual relative_path is `ext:ts:{package}/{sub_path}`.
pub(crate) fn walk_ts_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_ts_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

/// Scan the dep's source tree for files that declare `type_name` as a
/// class/interface/type/enum/function. Used by `resolve_symbol` to pull in
/// just the file(s) the chain walker needs, without dumping every file in
/// the dep into the index.
///
/// Caps total files scanned at `MAX_FILES_SCANNED` to bound worst-case cost
/// on huge declaration bundles (e.g. material-ui ships thousands of .d.ts
/// files). When the cap fires, callers fall back to the entry walk.
pub(crate) fn find_files_declaring_type(dep: &ExternalDepRoot, type_name: &str) -> Vec<WalkedFile> {
    const MAX_FILES_SCANNED: usize = 500;
    let mut out = Vec::new();
    let mut scanned = 0usize;
    scan_for_type_decl(&dep.root, &dep.root, dep, type_name, &mut out, &mut scanned, 0);
    if scanned >= MAX_FILES_SCANNED {
        // Bail out — search exceeded budget. Caller falls back to the
        // package entry walk.
        return Vec::new();
    }
    out
}

pub(crate) fn scan_for_type_decl(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    type_name: &str,
    out: &mut Vec<WalkedFile>,
    scanned: &mut usize,
    depth: u32,
) {
    const MAX_FILES_SCANNED: usize = 500;
    if depth >= MAX_WALK_DEPTH || *scanned >= MAX_FILES_SCANNED { return }

    let walk_nested = std::env::var_os("BEARWISDOM_TS_WALK_NESTED")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);

    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *scanned >= MAX_FILES_SCANNED { return }
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "node_modules" && !walk_nested { continue }
                // Skip every dot-prefixed directory: pnpm `.ignored_*`
                // shadows, the `.pnpm/` store root if it ever leaks through,
                // `.git`, `.cache`, `.storybook`, `.next`, etc. None of
                // them carry source we want to index, and `.ignored_*`
                // specifically would otherwise produce broken `ext:ts:`
                // paths whose package prefix can't be parsed.
                if name.starts_with('.') { continue }
                if matches!(
                    name,
                    "__tests__" | "__mocks__" | "test" | "tests" | "docs"
                        | "example" | "examples" | "_examples" | "fixtures"
                ) { continue }
            }
            scan_for_type_decl(&path, root, dep, type_name, out, scanned, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !is_ts_source_file(name) { continue }
            if is_test_or_story_file(name) { continue }

            *scanned += 1;
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            if !file_declares_type(&content, type_name) { continue }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => normalize_virtual_rel(&p.to_string_lossy()),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:ts:{}/{}", dep.module_path, rel_sub);
            let language = if name.ends_with(".tsx") { "tsx" } else { "typescript" };
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

/// Heuristic: does `content` declare `type_name` at the top level as a
/// class/interface/type/enum/function/const? The chain walker asks for
/// methods/fields *of* this type, so we only need the file that owns the
/// declaration; once parsed, the symbol's qualified name + members will be
/// in the index.
///
/// Patterns matched (whitespace-flexible):
///   `class Foo`, `interface Foo`, `type Foo`, `enum Foo`, `function Foo`,
///   `const Foo`, `let Foo`, `var Foo`
/// Each preceded by optional `export`/`declare`/`abstract`/`default`
/// keywords and followed by `<`, ` `, `=`, `(`, `:`, `{`, `;`, `\n`, or end.
pub(crate) fn file_declares_type(content: &str, type_name: &str) -> bool {
    // Cheap pre-filter: skip files that don't contain the name at all.
    if !content.contains(type_name) { return false }

    for raw in content.lines() {
        let line = raw.trim_start();
        // Strip combinations of leading modifiers; order doesn't matter.
        let stripped = strip_decl_modifiers(line);
        for keyword in &["class ", "interface ", "type ", "enum ", "function ",
                         "const ", "let ", "var ", "abstract class "]
        {
            if let Some(rest) = stripped.strip_prefix(keyword) {
                let rest = rest.trim_start();
                if let Some(after_name) = rest.strip_prefix(type_name) {
                    let next = after_name.chars().next();
                    let ok = matches!(
                        next,
                        None | Some(' ') | Some('<') | Some('=') | Some('(')
                            | Some(':') | Some('{') | Some(';') | Some('\t')
                            | Some('\n') | Some('\r')
                    );
                    if ok { return true }
                }
            }
        }
    }
    false
}

/// Drop leading `export`/`default`/`declare`/`abstract`/`async` modifiers in
/// any order. Returns a slice into the original string.
pub(crate) fn strip_decl_modifiers(line: &str) -> &str {
    let mut s = line.trim_start();
    loop {
        let mut advanced = false;
        for keyword in &["export ", "default ", "declare ", "async "] {
            if let Some(rest) = s.strip_prefix(keyword) {
                s = rest.trim_start();
                advanced = true;
                break;
            }
        }
        if !advanced { break }
    }
    s
}

pub(crate) fn walk_ts_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let walk_nested = std::env::var_os("BEARWISDOM_TS_WALK_NESTED")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);

    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "node_modules" && !walk_nested { continue }
                // Skip every dot-prefixed directory: pnpm `.ignored_*`
                // shadows, the `.pnpm/` store root if it ever leaks through,
                // `.git`, `.cache`, `.storybook`, `.next`, etc. None of
                // them carry source we want to index, and `.ignored_*`
                // specifically would otherwise produce broken `ext:ts:`
                // paths whose package prefix can't be parsed.
                if name.starts_with('.') { continue }
                if matches!(
                    name,
                    "__tests__" | "__mocks__" | "test" | "tests" | "docs"
                        | "example" | "examples" | "_examples" | "fixtures"
                ) { continue }
            }
            walk_ts_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !is_ts_source_file(name) { continue }
            if is_test_or_story_file(name) { continue }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => normalize_virtual_rel(&p.to_string_lossy()),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:ts:{}/{}", dep.module_path, rel_sub);
            let language = if name.ends_with(".tsx") { "tsx" } else { "typescript" };
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

/// Locate a package's type-declaration entry point using its `package.json`.
///
/// Priority:
///   1. `types` field (modern)
///   2. `typings` field (legacy alias of `types`)
///   3. `main` field with `.js`/`.mjs`/`.cjs` rewritten to the matching
///      `.d.ts` sibling if one exists on disk
///   4. Conventional fallbacks — `index.d.ts`, `dist/index.d.ts`, `lib/index.d.ts`
///
/// Returns the entry file plus any files it re-exports from WITHIN the same
/// dep root, bounded at depth `REEXPORT_MAX_DEPTH`. Without within-package
/// re-export expansion, entry-only parsing leaves most declaration bundles
/// opaque (vitest's `index.d.ts` is almost entirely `export { X } from
/// './chunks/...'` statements).
pub(crate) fn resolve_package_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let Some(entry) = resolve_package_entry_path(dep) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    expand_reexports_into(dep, &entry, &mut out, &mut seen, 0);
    out
}

pub(crate) fn resolve_package_entry_path(dep: &ExternalDepRoot) -> Option<PathBuf> {
    let pkg_json_path = dep.root.join("package.json");
    let json_str = std::fs::read_to_string(&pkg_json_path).ok();
    let parsed: Option<serde_json::Value> = json_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(pj) = parsed.as_ref() {
        // Modern conditional exports — `"exports"` field per Node.js
        // package-entry-points spec. When present, this wins over the
        // legacy `types`/`typings`/`main` fields because publishers use
        // it to point bundlers at differently-shaped artifacts (separate
        // ESM/CJS bundles, different .d.mts/.d.cts type files per
        // condition). The `types` condition is what we want — TypeScript
        // resolves type info through it, and so do we.
        if let Some(exports) = pj.get("exports") {
            if let Some(rel) = resolve_exports_types(exports) {
                candidates.push(dep.root.join(rel.trim_start_matches("./")));
            }
        }
        for field in ["types", "typings"] {
            if let Some(v) = pj.get(field).and_then(|v| v.as_str()) {
                candidates.push(dep.root.join(v.trim_start_matches("./")));
            }
        }
        if let Some(main) = pj.get("main").and_then(|v| v.as_str()) {
            let main_path = dep.root.join(main.trim_start_matches("./"));
            if main.ends_with(".d.ts") {
                candidates.push(main_path);
            } else {
                let stem = main_path.to_string_lossy().to_string();
                for ext in [".js", ".mjs", ".cjs"] {
                    if stem.ends_with(ext) {
                        let dts = stem.trim_end_matches(ext).to_string() + ".d.ts";
                        candidates.push(PathBuf::from(dts));
                        break;
                    }
                }
            }
        }
    }

    for fallback in ["index.d.ts", "dist/index.d.ts", "lib/index.d.ts", "types/index.d.ts"] {
        candidates.push(dep.root.join(fallback));
    }

    candidates.into_iter().find(|p| p.is_file())
}

/// Walk the `exports` field of a package.json looking for the `types`
/// condition that names the package's `.d.ts` entry.
///
/// Three top-level shapes per Node.js spec:
///   * Sugar string — `"exports": "./dist/index.js"`. No types info,
///     return `None`.
///   * Subpath map — `"exports": { ".": ..., "./sub": ... }`. Read the
///     `"."` entry as the root condition tree.
///   * Direct condition map — `"exports": { "types": "...", "import": ... }`.
///     The whole object is the root condition tree.
///
/// Within the condition tree, `types` (and the older `typings` synonym)
/// always wins. When `types` is itself an object, recurse — modern
/// publishers nest it under `import`/`require` to differentiate
/// `.d.mts`/`.d.cts`. Other condition keys (`node`, `import`, `require`,
/// `default`, `browser`) are walked as a fallback in case a publisher
/// only nested `types` inside one of them.
pub(crate) fn resolve_exports_types(exports: &serde_json::Value) -> Option<String> {
    let obj = exports.as_object()?;
    let is_subpath_map = obj.keys().any(|k| k == "." || k.starts_with("./"));
    let root = if is_subpath_map {
        obj.get(".")?
    } else {
        exports
    };
    extract_types_from_conditions(root)
}

pub(crate) fn extract_types_from_conditions(v: &serde_json::Value) -> Option<String> {
    let obj = v.as_object()?;
    for key in ["types", "typings"] {
        if let Some(child) = obj.get(key) {
            if let Some(s) = child.as_str() {
                return Some(s.to_string());
            }
            if let Some(s) = extract_types_from_conditions(child) {
                return Some(s);
            }
        }
    }
    // No direct `types` — look for it nested under conditional siblings
    // ordered most-likely-first. Stops at the first hit.
    for cond in ["node", "import", "require", "default", "browser"] {
        if let Some(child) = obj.get(cond) {
            if let Some(s) = extract_types_from_conditions(child) {
                return Some(s);
            }
        }
    }
    None
}

const REEXPORT_MAX_DEPTH: u32 = 3;

/// Header-scan walker: yield ONLY the package's type-entry file and the
/// in-package files reachable from it through relative re-exports. Used by
/// `build_npm_symbol_index` to keep the symbol-index build cost bounded
/// to per-dep entry traversal instead of the full source tree.
///
/// Returns empty when the package has no resolvable entry (rare — usually
/// a side-effect-only package). The dep root still participates in the
/// reachability loop via `resolve_import` and `resolve_symbol`, which
/// fall back to scanning the tree on demand.
pub(crate) fn walk_ts_dep_entry_only(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let Some(entry) = resolve_package_entry_path(dep) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    expand_reexports_into(dep, &entry, &mut out, &mut seen, 0);
    out
}

pub(crate) fn expand_reexports_into(
    dep: &ExternalDepRoot,
    file: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !seen.insert(file.to_path_buf()) { return }
    if !file.is_file() { return }
    let Ok(rel) = file.strip_prefix(&dep.root) else { return };
    // `resolve_relative_ts_path` joins specs like `./internal/foo` against
    // the parent dir without normalising, so `rel` can carry embedded `/./`
    // segments through to the virtual path. Collapse them here so the same
    // file emits a single canonical `ext:ts:<pkg>/dist/types/internal/Foo.d.ts`
    // shape regardless of which re-export hop pulled it in.
    let rel_s = normalize_virtual_rel(&rel.to_string_lossy());
    let lang = if rel_s.ends_with(".tsx") || rel_s.ends_with(".jsx") { "tsx" } else { "typescript" };
    out.push(WalkedFile {
        relative_path: format!("ext:ts:{}/{}", dep.module_path, rel_s),
        absolute_path: file.to_path_buf(),
        language: lang,
    });

    if depth >= REEXPORT_MAX_DEPTH { return }

    let Ok(src) = std::fs::read_to_string(file) else { return };
    for target in extract_relative_reexports(&src) {
        let Some(next) = resolve_relative_ts_path(file, &target) else { continue };
        expand_reexports_into(dep, &next, out, seen, depth + 1);
    }
}

/// Scan line-by-line for `export ... from '...'` and `import ... from '...'`
/// with relative specifiers. Returns the relative path strings in order.
/// Non-relative specifiers are skipped — they're separate packages with
/// their own dep roots.
pub(crate) fn extract_relative_reexports(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if !(t.starts_with("export") || t.starts_with("import")) { continue }
        let Some(ix) = t.find(" from ") else { continue };
        let rest = t[ix + 6..].trim_start();
        let Some(quote) = rest.chars().next() else { continue };
        if quote != '\'' && quote != '"' { continue }
        let inner = &rest[1..];
        if let Some(end) = inner.find(quote) {
            let spec = &inner[..end];
            if spec.starts_with("./") || spec.starts_with("../") {
                out.push(spec.to_string());
            }
        }
    }
    out
}

pub(crate) fn resolve_relative_ts_path(from_file: &Path, spec: &str) -> Option<PathBuf> {
    let base = from_file.parent()?;
    let raw = base.join(spec);
    let raw_str = raw.to_string_lossy().to_string();

    // Rollup-bundled type-entry shells re-export from `./chunk.js`-style
    // companions (vue-router 5.0.4's `dist/vue-router.d.ts` reads
    // `from "./index-BzEKChPW.js"`, with the actual types at
    // `index-BzEKChPW.d.ts`). The naïve append-`.d.ts` path misses these
    // because it would produce `index-BzEKChPW.js.d.ts`. Strip the runtime
    // extension first and probe the matching declarations companion.
    for (runtime_ext, type_exts) in [
        (".js", [".d.ts", ".d.mts", ".d.cts"]),
        (".mjs", [".d.mts", ".d.ts", ".d.cts"]),
        (".cjs", [".d.cts", ".d.ts", ".d.mts"]),
    ] {
        if let Some(stripped) = raw_str.strip_suffix(runtime_ext) {
            for type_ext in type_exts {
                let p = PathBuf::from(format!("{stripped}{type_ext}"));
                if p.is_file() { return Some(p) }
            }
        }
    }

    for ext in [".d.ts", ".ts", ".tsx", ".mts", ".cts", ".d.mts", ".d.cts"] {
        let p = PathBuf::from(format!("{raw_str}{ext}"));
        if p.is_file() { return Some(p) }
    }
    for ext in ["index.d.ts", "index.ts", "index.tsx", "index.d.mts", "index.d.cts"] {
        let p = raw.join(ext);
        if p.is_file() { return Some(p) }
    }
    if raw.is_file() { return Some(raw) }
    None
}

pub(crate) fn is_ts_source_file(name: &str) -> bool {
    name.ends_with(".ts")
        || name.ends_with(".tsx")
        || name.ends_with(".mts")
        || name.ends_with(".cts")
}

pub(crate) fn is_test_or_story_file(name: &str) -> bool {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    stem.ends_with(".test")
        || stem.ends_with(".spec")
        || stem.ends_with(".stories")
        || stem.ends_with(".bench")
        || stem.ends_with(".fixture")
        || stem == "test"
        || stem == "index.test"
}

// ---------------------------------------------------------------------------
// Post-process: prefix declaration-file symbols with their package name
// ---------------------------------------------------------------------------

/// Prefix every symbol's `qualified_name` (and `scope_path`) in a parsed
/// TypeScript external file with the owning package name.
///
/// TypeScript declaration files don't carry a package-level scope, so the
/// extractor yields bare qualified names like `Button`. Rewrite them to
/// `fake-ui.Button` so the TS resolver's `{import_module}.{target}` lookup
/// matches. Idempotent: already-prefixed names are left alone.
/// Re-scan `source` for `declare global { ... }` blocks and inject a
/// `SymbolKind::Variable` entry per declared name that the TypeScript
/// extractor missed. The extractor's ambient-block descent is incomplete
/// (only a subset of const/let/function/class decls inside declare global
/// surface as top-level symbols), which starves the heuristic resolver's
/// declare-global priority of the files it relies on. Re-scanning at
/// post-process time is the narrowest correctness fix.
pub(crate) fn backfill_declare_global_symbols(pf: &mut crate::types::ParsedFile, source: &str) {
    use crate::types::{ExtractedSymbol, SymbolKind};

    let globals = scan_declare_global_blocks(source);
    if globals.is_empty() {
        return;
    }
    let existing: std::collections::HashSet<String> =
        pf.symbols.iter().map(|s| s.name.clone()).collect();
    let existing_qnames: std::collections::HashSet<String> =
        pf.symbols.iter().map(|s| s.qualified_name.clone()).collect();
    for name in globals {
        // Dotted names (`Express.Multer.File`) are namespace paths whose
        // inner symbols the TS extractor already lifts as proper
        // class/interface/namespace symbols at the right qname. Emitting a
        // synthetic Variable here would only duplicate them under a name
        // the heuristic resolver's qname-derived index still wouldn't key
        // on (it uses the qname's last segment, not `sym.name`). Restrict
        // the backfill to flat top-level decls (`expect`, `describe`,
        // `Buffer`, `process`) where the extractor's ambient-block descent
        // is the unreliable bit.
        if name.contains('.') {
            continue;
        }
        // Primary entry — the symbol the package owns. After
        // prefix_ts_external_symbols runs, this becomes
        // `<package>.<name>` so package-qualified lookups
        // (`@angular/localize.$localize`) match.
        if !existing.contains(&name) {
            pf.symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name.clone(),
                kind: SymbolKind::Variable,
                visibility: None,
                start_line: 0,
                end_line: 0,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index: None,
            });
        }
        // Shadow entry under the synthetic globals namespace so the
        // resolver's bare-name fallback (`ts_npm_globals` strategy in
        // languages/typescript/resolve.rs) can find globals like
        // `$localize`, `describe`, `it`, `cy`, `expect` that the source
        // references without an explicit `import`. The shadow is a
        // separate symbol so the package-prefix pass below leaves its
        // qname intact (it short-circuits when the qname already starts
        // with the package prefix; we add a special-case for the globals
        // namespace right next to that check).
        let shadow_qname = format!("{NPM_GLOBALS_MODULE}.{name}");
        if !existing_qnames.contains(&shadow_qname) {
            pf.symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: shadow_qname,
                kind: SymbolKind::Variable,
                visibility: None,
                start_line: 0,
                end_line: 0,
                start_col: 0,
                end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index: None,
            });
        }
    }
}

/// Post-process a TS external file pulled through the demand-driven path.
/// Mirrors what the eager-walk locator's `post_process_parsed` does: scan
/// for `declare global` / `declare namespace` blocks and inject any names
/// the extractor missed, then prefix every symbol's qname with the owning
/// package. Both demand-driven entry points (`stage_link::seed_demand_*`
/// and `expand::expand_*`) must call this so the symbol table is shaped
/// the same regardless of which pass pulled the file in.
pub(crate) fn ts_post_process_external(pf: &mut crate::types::ParsedFile) {
    let Some(pkg) = ts_package_from_virtual_path(&pf.path).map(str::to_string) else {
        return;
    };
    // TS core lib (lib.dom.d.ts, lib.es*.d.ts, …) declares runtime globals
    // — `HTMLElement`, `Document`, `Promise`, etc. — at ambient scope.
    // Prefixing them under a synthetic package would mangle their qnames
    // away from the bare names the chain walker queries, so we skip the
    // post-processing entirely for files served from the synthetic
    // `__ts_lib__` module. Backfill is also a no-op for these — they
    // don't carry `declare global { … }` blocks.
    if pkg == crate::ecosystem::ts_lib_dom::TS_LIB_SYNTHETIC_MODULE {
        return;
    }
    let source_snapshot = pf.content.clone();
    if let Some(source) = source_snapshot.as_deref() {
        if source.contains("declare global") || source.contains("declare namespace") {
            backfill_declare_global_symbols(pf, source);
        }
    }
    prefix_ts_external_symbols(pf, &pkg);
}

pub(crate) fn prefix_ts_external_symbols(pf: &mut crate::types::ParsedFile, package: &str) {
    if package.is_empty() { return }
    let prefix = format!("{package}.");
    let globals_prefix = format!("{NPM_GLOBALS_MODULE}.");
    for sym in &mut pf.symbols {
        // Shadow symbols pushed by `backfill_declare_global_symbols` carry
        // the synthetic globals qname so the resolver's bare-name fallback
        // can find them. Don't tack the package prefix in front — that
        // would mangle the namespace key the resolver looks up.
        if sym.qualified_name.starts_with(&globals_prefix) {
            continue;
        }
        if !sym.qualified_name.starts_with(&prefix) {
            sym.qualified_name = format!("{prefix}{}", sym.qualified_name);
        }
        sym.scope_path = match sym.scope_path.take() {
            Some(sp) if !sp.starts_with(&prefix) => Some(format!("{prefix}{sp}")),
            Some(sp) => Some(sp),
            None => Some(package.to_string()),
        };
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------
//
// Walks every reached npm dep root, tree-sitter parses each TS/JS source
// file without descending into function/method/class bodies, and records
// each top-level declaration's name against the file that defines it. The
// Stage 2 loop consults this index to pull only files it needs; the
// (gigabytes of) `node_modules/` that the eager walker used to force-parse
// stays untouched unless a real user chain lands on one of its symbols.
//
// File scope matches `walk_ts_external_root` — same .ts/.tsx/.d.ts/.js/.jsx
// filter, same exclusions (nested node_modules, test/story dirs, etc.).

/// Synthetic module key under which `declare global { ... }` names get
/// indexed. Resolvers doing a bare-name fallback for unimported globals
/// (vitest's `describe`/`it`/`expect` when `globals: true`, `@types/jest`
/// globals, `@types/node` `process`/`Buffer`, etc.) look up
/// `(__NPM_GLOBALS__, name)`.
pub(crate) const NPM_GLOBALS_MODULE: &str = "__npm_globals__";

pub(crate) fn build_npm_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    // Entry-only header scan: resolve each dep's type-entry file (via
    // package.json `types`/`exports`/`main` ↔ `.d.ts` companion) and
    // follow its relative re-exports up to REEXPORT_MAX_DEPTH. Files
    // reachable only through deep import paths
    // (e.g. `import x from 'rxjs/operators'`) stay OUT of the symbol
    // index — `resolve_symbol`'s `find_files_declaring_type` fallback
    // pulls them on-demand when the chain walker asks for a name that
    // isn't indexed.
    //
    // Full-tree walks were the dominant cost of npm externals indexing:
    // material-ui ships ~3 K declaration files, lodash/rxjs/three.js
    // similar — almost all of them unreachable from the user's actual
    // imports. Globals-declaring packages bypass the entry restriction:
    // their `declare global { ... }` blocks anywhere in the tree need
    // to surface as global symbols, so `package_declares_globals` gates
    // a full walk instead of entry-only.
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        let walked = if package_declares_globals(&dep.root) {
            walk_ts_external_root(dep)
        } else {
            walk_ts_dep_entry_only(dep)
        };
        for wf in walked {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    // Parallel header-only scan. Each task returns a FileExports record
    // capturing (a) which names the file defines locally, (b) which names
    // it re-exports and from where, (c) wildcard `export * from 'x'`
    // sources, and (d) `declare global { ... }` names.
    let scanned: Vec<(String, PathBuf, FileExports)> = work
        .par_iter()
        .map(|(module, wf)| {
            let exports = std::fs::read_to_string(&wf.absolute_path)
                .ok()
                .map(|src| scan_ts_file_exports(&src, wf.language))
                .unwrap_or_default();
            (module.clone(), wf.absolute_path.clone(), exports)
        })
        .collect();

    // Build a by-path view for re-export resolution. Two structures: a
    // HashSet<PathBuf> of every scanned file (for `resolve_relative_in_set`
    // so we don't hit the filesystem per edge) and a HashMap<Path, &exports>
    // so resolve_definition can follow named re-exports through the graph.
    let known_paths: HashSet<PathBuf> =
        scanned.iter().map(|(_, p, _)| p.clone()).collect();
    let by_path: HashMap<&Path, &FileExports> = scanned
        .iter()
        .map(|(_, p, e)| (p.as_path(), e))
        .collect();

    let mut index = SymbolLocationIndex::new();
    for (module, file, exports) in &scanned {
        // Globals: indexed under BOTH the synthetic globals module (so
        // bare-name fallback finds them) and the owning package (so
        // package-qualified lookups still resolve). Unchanged from before.
        for g in &exports.globals {
            index.insert(NPM_GLOBALS_MODULE, g.clone(), file.clone());
            index.insert(module, g.clone(), file.clone());
        }

        // Named exports: resolve each to its DEFINITION file by walking
        // the re-export graph. A barrel like `axios/index.d.ts` that
        // does `export { get } from './core'` resolves 'get' to
        // './core.d.ts' so `locate('axios', 'get')` points at the real
        // definition instead of the barrel.
        for (exposed, source) in &exports.named {
            let mut visited = HashSet::new();
            let def_file = resolve_definition(
                &by_path,
                &known_paths,
                file,
                source,
                &mut visited,
            )
            .unwrap_or_else(|| file.clone());
            index.insert(module, exposed.clone(), def_file);
        }

        // Wildcards: `export * from './mod'`. Collect every name exposed
        // through the wildcard chain (recursive, cycle-guarded) and
        // register each under OUR package's module with the definition
        // file. Cross-package wildcards (`export * from 'other-pkg'`)
        // are skipped — the other package's scan already indexes those
        // names under its own module, and we don't want to double-count.
        let mut wc_seen: HashSet<PathBuf> = HashSet::new();
        let mut wc_names: HashMap<String, PathBuf> = HashMap::new();
        for wc in &exports.wildcards {
            if !wc.starts_with('.') {
                continue;
            }
            let Some(parent) = file.parent() else { continue };
            let Some(wc_path) = resolve_relative_in_set(parent, wc, &known_paths) else {
                continue;
            };
            collect_wildcard_names(
                &by_path,
                &known_paths,
                &wc_path,
                &mut wc_seen,
                &mut wc_names,
            );
        }
        for (name, def_file) in wc_names {
            index.insert(module, name, def_file);
        }
    }
    index
}

// ---------------------------------------------------------------------------
// Re-export resolution helpers (index-build time)
// ---------------------------------------------------------------------------

/// Follow a (potentially chained) re-export from `current_file` to the file
/// that actually defines the symbol. Returns `None` when the chain exits the
/// same-package scope (cross-package specifier), dead-ends in an unscanned
/// file, or hits a cycle.
///
/// Callers fall back to indexing the name at the barrel file on `None`,
/// preserving pre-refactor behaviour for cases we can't follow statically.
pub(crate) fn resolve_definition(
    by_path: &HashMap<&Path, &FileExports>,
    known_paths: &HashSet<PathBuf>,
    current_file: &Path,
    source: &ExportSource,
    visited: &mut HashSet<(PathBuf, String)>,
) -> Option<PathBuf> {
    match source {
        ExportSource::Local => Some(current_file.to_path_buf()),
        ExportSource::Namespace { module } => {
            // `export * as ns from './mod'` — ns points at the whole
            // module's entry file. No single "original" symbol name to
            // follow through chains; resolving terminates at the module
            // file itself. Cross-package namespace re-exports return
            // None with the same reasoning as cross-package Reexports.
            if !module.starts_with('.') {
                return None;
            }
            let parent = current_file.parent()?;
            resolve_relative_in_set(parent, module, known_paths)
        }
        ExportSource::Reexport { module, original } => {
            if !module.starts_with('.') {
                // Cross-package — the target package's scan indexes its
                // own Locals under its own module, so `locate(target_pkg,
                // original)` already answers for the user. We can't
                // bridge pkg-A's re-export of pkg-B's symbol into a
                // unified pointer without the pkg-B index in hand, and
                // that lives in a different ecosystem's dep_roots.
                return None;
            }
            let parent = current_file.parent()?;
            let target = resolve_relative_in_set(parent, module, known_paths)?;
            if !visited.insert((target.clone(), original.clone())) {
                return None;
            }
            let target_exports = by_path.get(target.as_path())?;
            if let Some(inner) = target_exports.named.get(original) {
                return resolve_definition(
                    by_path,
                    known_paths,
                    &target,
                    inner,
                    visited,
                );
            }
            // Name not directly in target.named — try wildcard re-exports
            // in the target file. `export * from './sub'` surfaces every
            // name in sub under the current file's export set.
            for wc in &target_exports.wildcards {
                if !wc.starts_with('.') {
                    continue;
                }
                let Some(wc_parent) = target.parent() else { continue };
                let Some(wc_path) =
                    resolve_relative_in_set(wc_parent, wc, known_paths)
                else {
                    continue;
                };
                let Some(wc_exports) = by_path.get(wc_path.as_path()) else {
                    continue;
                };
                if let Some(inner) = wc_exports.named.get(original) {
                    if let Some(def) = resolve_definition(
                        by_path,
                        known_paths,
                        &wc_path,
                        inner,
                        visited,
                    ) {
                        return Some(def);
                    }
                }
            }
            None
        }
    }
}

/// Gather every name reachable through `export * from` starting at `file`,
/// mapping each to its definition path. Wildcards chain through nested
/// wildcards too. `seen` guards cycles; `out` accumulates names with
/// first-writer-wins semantics (consistent with the index at large).
pub(crate) fn collect_wildcard_names(
    by_path: &HashMap<&Path, &FileExports>,
    known_paths: &HashSet<PathBuf>,
    file: &Path,
    seen: &mut HashSet<PathBuf>,
    out: &mut HashMap<String, PathBuf>,
) {
    if !seen.insert(file.to_path_buf()) {
        return;
    }
    let Some(exports) = by_path.get(file) else { return };
    for (name, source) in &exports.named {
        if out.contains_key(name) {
            continue;
        }
        let mut visited = HashSet::new();
        let def_file = resolve_definition(by_path, known_paths, file, source, &mut visited)
            .unwrap_or_else(|| file.to_path_buf());
        out.insert(name.clone(), def_file);
    }
    for wc in &exports.wildcards {
        if !wc.starts_with('.') {
            continue;
        }
        let Some(parent) = file.parent() else { continue };
        let Some(wc_path) = resolve_relative_in_set(parent, wc, known_paths) else {
            continue;
        };
        collect_wildcard_names(by_path, known_paths, &wc_path, seen, out);
    }
}

/// Filesystem-free variant of `resolve_ts_relative_import`: probes the set
/// of already-scanned files (with the same extension + index resolution
/// rules) instead of hitting disk. Saves millions of `is_file` syscalls
/// on large `node_modules` trees.
pub(crate) fn resolve_relative_in_set(
    base_dir: &Path,
    specifier: &str,
    known: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let target = base_dir.join(specifier);
    const EXTS: &[&str] = &[
        "ts", "tsx", "d.ts", "mts", "cts", "js", "jsx", "mjs", "cjs",
    ];
    for ext in EXTS {
        let candidate = target.with_extension(ext);
        if known.contains(&candidate) {
            return Some(candidate);
        }
    }
    for ext in EXTS {
        let candidate = target.join(format!("index.{ext}"));
        if known.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// How an exposed name in a TS/JS file gets its value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExportSource {
    /// `function X() {}`, `class X {}`, `export { localX }`, etc. — X is
    /// defined in this file.
    Local,
    /// `export { Orig as Exposed } from 'module'` — the exposed name is
    /// sourced from `module`, under `original_name` (which equals the
    /// exposed name when there's no renaming).
    Reexport { module: String, original: String },
    /// `export * as ns from 'module'` — the exposed name is the whole
    /// module's namespace object. Resolves to the module's entry file;
    /// there's no single `original` symbol name to track because the
    /// namespace is the aggregate of every export in `module`.
    Namespace { module: String },
}

/// Per-file export summary built by `scan_ts_file_exports`. Keys of
/// `named` are exposed names; the enum tells us whether a name is a
/// definition or a re-export so the index builder can follow re-exports
/// to the file that actually defines the symbol.
#[derive(Debug, Default, Clone)]
struct FileExports {
    /// exposed_name → where the value comes from
    named: HashMap<String, ExportSource>,
    /// Module specifiers of `export * from '<module>'` statements.
    wildcards: Vec<String>,
    /// `declare global { ... }` names — surfaced separately (pollute the
    /// global namespace regardless of any import).
    globals: Vec<String>,
}

/// Header-only tree-sitter scan of a TS/TSX/JS source file. Returns a
/// FileExports describing:
///
/// - `named`: top-level decls and named exports. Each keyed by the name
///   *as exposed by this file* and tagged with whether it's defined here
///   (Local) or re-exported from another module (Reexport).
/// - `wildcards`: `export * from 'x'` module specifiers.
/// - `globals`: names declared inside `declare global { ... }` blocks.
///
/// Function/method/class bodies are not walked. DefinitelyTyped shapes
/// (`declare module 'foo' { ... }`) surface their inner decls as Local
/// under the ambient module name of the containing file.
pub(crate) fn scan_ts_file_exports(source: &str, language: &str) -> FileExports {
    let mut out = FileExports::default();

    let ts_lang: tree_sitter::Language = match language {
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    };
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(source, None) else {
        return out;
    };

    let root = tree.root_node();
    let bytes = source.as_bytes();

    // First pass: build a local→(module, original_name) import map so the
    // export pass can decide whether `export { X }` (no `from`) is a
    // genuine local forward or a re-export of an imported binding. Without
    // this, `import { X } from './mod'; export { X }` was misrecorded as
    // Local pointing at the barrel file — breaking any downstream lookup
    // that expected X's definition to be in `./mod`.
    let mut imports: HashMap<String, (String, String)> = HashMap::new();
    {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            collect_imports(&child, bytes, &mut imports);
        }
    }

    // Second pass: exports + wildcards + namespace re-exports.
    {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            collect_file_exports(&child, bytes, &mut out, &imports);
        }
    }

    // `declare global { ... }` extraction: tree-sitter-typescript's grammar
    // wraps this inconsistently across minor grammar releases, so fall back
    // to a regex sweep of the source.
    out.globals = scan_declare_global_blocks(source);

    // `declare module 'vue' { interface GlobalComponents { ... } }` —
    // member names are auto-registered as global Vue template components
    // by `app.use(<plugin>)`. Source forms covered:
    //
    //   declare module 'vue' { interface GlobalComponents { NButton: ...; } }
    //   declare module '@vue/runtime-core' { interface GlobalComponents { RouterLink: ...; } }
    //   declare module 'vue/types/vue' { interface GlobalComponents { ... } }
    //
    // Lifted into `out.globals` so the existing `__npm_globals__` demand-pull
    // path and heuristic ambient-path priority pick them up identically to
    // `declare global { const expect; }`-style names.
    if source.contains("GlobalComponents") {
        out.globals.extend(scan_vue_global_components(source));
    }

    out
}

/// Extract member names from `declare module 'vue' { interface GlobalComponents { ... } }`
/// and equivalent augmentations (`@vue/runtime-core`, `vue/types/vue`).
/// Each property declaration inside the interface contributes its name.
///
/// Two member shapes are common:
///   - explicit-list (Naive UI volar.d.ts, Vue Router, Element Plus,
///     unplugin-vue-components-generated `components.d.ts`):
///     `NButton: (typeof import('naive-ui'))['NButton']`
///   - reference shape (Vue Router): `RouterLink: typeof RouterLink`
///
/// Both surface as a property whose name is the leftmost identifier — only
/// the name is extracted; the type expression is irrelevant for resolution
/// since the symbol gets pulled by name via `__npm_globals__`.
pub(crate) fn scan_vue_global_components(source: &str) -> Vec<String> {
    // Match `declare module '<vue-ish>'` opening braces.
    let module_re = regex::Regex::new(
        r#"declare\s+module\s+['"](?:vue|@vue/runtime-core|vue/types/vue)['"]\s*\{"#,
    )
    .expect("vue module regex");
    let bytes = source.as_bytes();

    let mut out: Vec<String> = Vec::new();
    for m in module_re.find_iter(source) {
        let open_brace = m.end() - 1;
        let Some(close) = find_matching_brace(bytes, open_brace) else { continue };
        let module_block = &source[open_brace + 1..close];

        // Find `interface GlobalComponents` (with optional `export`/`extends`)
        // inside the module block, then collect property names from its body.
        let iface_re = regex::Regex::new(
            r"(?:export\s+)?interface\s+GlobalComponents(?:\s+extends\s+[^{]+)?\s*\{",
        )
        .expect("globalcomponents interface regex");
        let iface_block_bytes = module_block.as_bytes();
        for cap in iface_re.find_iter(module_block) {
            let body_open = cap.end() - 1;
            let Some(body_close) = find_matching_brace(iface_block_bytes, body_open) else { continue };
            let body = &module_block[body_open + 1..body_close];
            // Property declarations: `Name: <type>` or `Name?: <type>`.
            // Skip nested braces (e.g. mapped types) by only matching at the
            // shallow level — naïve approach via line-anchored regex.
            let prop_re = regex::Regex::new(r"(?m)^\s*([A-Za-z_$][\w$]*)\s*\??\s*:")
                .expect("property regex");
            for prop_cap in prop_re.captures_iter(body) {
                out.push(prop_cap[1].to_string());
            }
        }
    }
    out
}

/// Populate `out` with every import binding surfaced by an
/// `import_statement` node. Entry shape: `local_name → (module, original)`.
///
/// - `import X from 'mod'`                    → `"X" → ("mod", "default")`
/// - `import { X, Y as Y2 } from 'mod'`       → `"X" → ("mod", "X")`, `"Y2" → ("mod", "Y")`
/// - `import * as ns from 'mod'`              → `"ns" → ("mod", "*")`
/// - `import 'side-effect'`                   → nothing
///
/// The sentinel `"*"` in the original-name slot is recognised by the
/// export pass and upgraded to an `ExportSource::Namespace` when the
/// binding is re-exported without a `from` clause.
pub(crate) fn collect_imports(
    node: &Node,
    bytes: &[u8],
    out: &mut HashMap<String, (String, String)>,
) {
    if node.kind() != "import_statement" {
        return;
    }
    let Some(module) = extract_export_source_module(node, bytes) else {
        return;
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "import_clause" {
            continue;
        }
        let mut cc = child.walk();
        for piece in child.children(&mut cc) {
            match piece.kind() {
                // `import X from 'mod'` — default import.
                "identifier" => {
                    if let Ok(name) = piece.utf8_text(bytes) {
                        out.insert(
                            name.to_string(),
                            (module.clone(), "default".to_string()),
                        );
                    }
                }
                // `import { X, Y as Y2 } from 'mod'`
                "named_imports" => {
                    let mut sc = piece.walk();
                    for spec in piece.children(&mut sc) {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let orig_node = spec.child_by_field_name("name");
                        let alias_node = spec.child_by_field_name("alias");
                        let local_node = alias_node.or(orig_node);
                        let (Some(on), Some(ln)) = (orig_node, local_node) else {
                            continue;
                        };
                        let Ok(original) = on.utf8_text(bytes) else { continue };
                        let Ok(local) = ln.utf8_text(bytes) else { continue };
                        out.insert(
                            local.to_string(),
                            (module.clone(), original.to_string()),
                        );
                    }
                }
                // `import * as ns from 'mod'`
                "namespace_import" => {
                    let mut sc = piece.walk();
                    for n in piece.children(&mut sc) {
                        if n.kind() == "identifier" {
                            if let Ok(name) = n.utf8_text(bytes) {
                                out.insert(
                                    name.to_string(),
                                    (module.clone(), "*".to_string()),
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Back-compat helper: the pre-refactor `scan_ts_header` returned
/// `(regular_names, global_names)` as flat vecs. Tests and a handful of
/// callers still depend on that shape. We derive it from the richer
/// FileExports so both surfaces stay in sync.
#[cfg(test)]
pub(crate) fn scan_ts_header(source: &str, language: &str) -> (Vec<String>, Vec<String>) {
    let exports = scan_ts_file_exports(source, language);
    let mut regular: Vec<String> = exports.named.into_keys().collect();
    regular.sort();
    (regular, exports.globals)
}

/// Extract names declared inside `declare global { ... }` and top-level
/// `declare namespace X { ... }` blocks. Returns a flat list including
/// dotted names for declarations nested inside `namespace` wrappers.
///
/// Examples of names emitted:
/// - `declare global { const expect; }` → `expect`
/// - `declare global { namespace Express { interface Request {} } }` → `Express`, `Express.Request`
/// - `declare namespace google.maps { class Map {} class LatLng {} }` → `google.maps.Map`, `google.maps.LatLng`
/// - `declare namespace google { namespace maps { class Map {} } }` → `google`, `google.maps.Map`
///
/// Source-scan approach (rather than tree-sitter) is grammar-independent
/// against tree-sitter-typescript's variance in how `global` and ambient
/// `namespace` wrappers land in the CST.
pub(crate) fn scan_declare_global_blocks(source: &str) -> Vec<String> {
    let has_global = source.contains("declare global");
    let has_ns = source.contains("declare namespace");
    if !has_global && !has_ns {
        return Vec::new();
    }
    let bytes = source.as_bytes();

    let mut out: Vec<String> = Vec::new();

    if has_global {
        let marker_re = regex::Regex::new(r"declare\s+global\s*\{").expect("declare global regex");
        for m in marker_re.find_iter(source) {
            // Opening `{` is the last char of the match.
            let open_brace = m.end() - 1;
            if let Some(close) = find_matching_brace(bytes, open_brace) {
                let block = &source[open_brace + 1..close];
                collect_namespace_decls("", block, &mut out);
            }
        }
    }

    if has_ns {
        // Top-level `declare namespace X.Y { ... }` and `declare namespace X { ... }`.
        // The namespace path can be dotted (e.g. `declare namespace google.maps`).
        let ns_re = regex::Regex::new(
            r"(?m)^\s*declare\s+namespace\s+([A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*)\s*\{",
        )
        .expect("declare namespace regex");
        for cap in ns_re.captures_iter(source) {
            let path: String = cap[1].chars().filter(|c| !c.is_whitespace()).collect();
            let m = cap.get(0).unwrap();
            let open_brace = m.end() - 1;
            if let Some(close) = find_matching_brace(bytes, open_brace) {
                let block = &source[open_brace + 1..close];
                out.push(path.clone());
                collect_namespace_decls(&path, block, &mut out);
            }
        }
    }

    out
}

/// Given a source byte offset pointing at an opening `{`, return the offset
/// of the matching `}`, or `None` if unbalanced. Naïve brace counter — does
/// not skip braces inside strings/comments, but `.d.ts` declaration files
/// don't realistically contain those at significant depth.
pub(crate) fn find_matching_brace(bytes: &[u8], open_brace: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut i = open_brace + 1;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Walk a block body, emitting names for each top-level declaration. When
/// a nested `namespace Y { ... }` appears, recurse with the prefix extended
/// (`prefix.Y`) so leaf decls land as `prefix.Y.Leaf`.
///
/// `prefix` is the current dotted namespace path (`""` at the outermost
/// `declare global` body). Decls at the current level are pushed as
/// `prefix.name` (or just `name` when prefix is empty). Inner namespace
/// wrapper names are pushed too, so a chain ref like `Express.Multer` (one
/// hop short of a leaf) still finds *something* in the index.
pub(crate) fn collect_namespace_decls(prefix: &str, block: &str, out: &mut Vec<String>) {
    // JS/TS identifiers allow `$` and `_` as the leading character (and
    // anywhere else). `\w` is `[A-Za-z0-9_]` and silently drops anything
    // starting with `$` — Angular's `$localize`, jQuery's `$`, lodash's
    // `_` (when not a namespace), Cypress's `cy` (only happens to be \w),
    // RxJS's `$`-suffix observables. Use the full JS identifier shape so
    // every globally-declared symbol that downstream projects reference
    // gets indexed.
    let decl_re = regex::Regex::new(
        r"(?m)^\s*(?:export\s+)?(?:const|let|var|function|class|abstract\s+class|type|interface|enum)\s+([A-Za-z_$][\w$]*)",
    )
    .expect("namespace decl regex");
    for cap in decl_re.captures_iter(block) {
        let name = &cap[1];
        out.push(if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        });
    }

    // Nested namespace wrappers: `namespace X { ... }` (with optional
    // `export`). Path can be dotted: `namespace X.Y { ... }`.
    let ns_re = regex::Regex::new(
        r"(?m)^\s*(?:export\s+)?namespace\s+([A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*)\s*\{",
    )
    .expect("nested namespace regex");
    let bytes = block.as_bytes();
    for cap in ns_re.captures_iter(block) {
        let path: String = cap[1].chars().filter(|c| !c.is_whitespace()).collect();
        let m = cap.get(0).unwrap();
        let open_brace = m.end() - 1;
        if let Some(close) = find_matching_brace(bytes, open_brace) {
            let inner = &block[open_brace + 1..close];
            let new_prefix = if prefix.is_empty() {
                path.clone()
            } else {
                format!("{prefix}.{path}")
            };
            // The wrapper name itself is also a useful index entry — chain
            // refs that stop one hop short of a leaf (`Express.Multer`)
            // still resolve to *something* rather than going unmatched.
            out.push(new_prefix.clone());
            collect_namespace_decls(&new_prefix, inner, out);
        }
    }
}

/// Inspect one direct child of the source-file root and record any top-level
/// declaration or export into `out`. Local definitions land as
/// `ExportSource::Local`; named re-exports (`export { X } from 'mod'`) land
/// as `ExportSource::Reexport`; star re-exports (`export * from 'mod'`) land
/// in `out.wildcards`. Recurses into `export_statement`, `ambient_declaration`,
/// and `internal_module`/`module` (namespace) wrappers. Does NOT recurse into
/// `block` / `statement_block` / `class_body` / `function_body` — bodies are
/// where header-only parsing draws the line.
pub(crate) fn collect_file_exports(
    node: &Node,
    bytes: &[u8],
    out: &mut FileExports,
    imports: &HashMap<String, (String, String)>,
) {
    match node.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "function_signature"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.named
                        .entry(name.to_string())
                        .or_insert(ExportSource::Local);
                }
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for decl in node.children(&mut cursor) {
                if decl.kind() == "variable_declarator" {
                    if let Some(name_node) = decl.child_by_field_name("name") {
                        if name_node.kind() == "identifier" {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                out.named
                                    .entry(name.to_string())
                                    .or_insert(ExportSource::Local);
                            }
                        }
                    }
                }
            }
        }
        "export_statement" => {
            // Source module (`from '<mod>'`) if present on this statement:
            // `export { X } from './m'`           →  source = Some("./m")
            // `export class Foo {}`                →  source = None
            // `export * from './m'`                →  source = Some("./m")
            // `export * as ns from './m'`          →  source = Some("./m"), namespace re-export
            let source_module = extract_export_source_module(node, bytes);

            // Scan direct children once for the three distinct shapes we need:
            //   `*` alone                 → wildcard re-export
            //   `*` + namespace_export    → named namespace re-export
            //   `default` keyword         → default export (sibling may be a decl or identifier)
            let mut has_star = false;
            let mut namespace_name: Option<String> = None;
            let mut has_default_keyword = false;
            {
                let mut cursor = node.walk();
                for ch in node.children(&mut cursor) {
                    match ch.kind() {
                        "*" => has_star = true,
                        "default" => has_default_keyword = true,
                        "namespace_export" => {
                            // `* as ns` — the identifier lives inside.
                            let mut nc = ch.walk();
                            for nch in ch.children(&mut nc) {
                                if nch.kind() == "identifier" {
                                    if let Ok(n) = nch.utf8_text(bytes) {
                                        namespace_name = Some(n.to_string());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            if has_star {
                if let Some(src) = source_module.clone() {
                    if let Some(ns) = namespace_name {
                        // `export * as ns from './mod'` — single named
                        // export bound to the whole module namespace.
                        out.named
                            .entry(ns)
                            .or_insert(ExportSource::Namespace { module: src });
                    } else {
                        // `export * from './mod'` — pure wildcard.
                        out.wildcards.push(src);
                    }
                }
                // Star statement has no further specifiers to process.
                return;
            }

            // `export default ...` — register a synthetic `default` entry
            // so downstream `export { default as X } from './this'`
            // re-exports in other files can resolve through the index.
            // Points at this file: the wrapped declaration, object literal,
            // or expression is what the default value resolves to.
            if has_default_keyword {
                out.named
                    .entry("default".to_string())
                    .or_insert(ExportSource::Local);
            }

            // Walk children: each export_clause specifier is a (re-)export;
            // other children are wrapped local decls to recurse into.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                match inner.kind() {
                    "export_clause" => {
                        let mut cc = inner.walk();
                        for spec in inner.children(&mut cc) {
                            if spec.kind() != "export_specifier" {
                                continue;
                            }
                            let orig_node = spec.child_by_field_name("name");
                            let alias_node = spec.child_by_field_name("alias");
                            let exposed_node = alias_node.or(orig_node);
                            let (Some(on), Some(en)) = (orig_node, exposed_node) else {
                                continue;
                            };
                            let Ok(original) = on.utf8_text(bytes) else { continue };
                            let Ok(exposed) = en.utf8_text(bytes) else { continue };
                            // Three cases for the source:
                            //   (a) `export { X } from 'mod'` — direct re-export.
                            //   (b) `export { X }` with no `from`, but X was
                            //       imported in this file — transitive re-export.
                            //       Without this branch, the index would record
                            //       X as Local to the barrel and lose the
                            //       connection to the real definition file.
                            //   (c) `export { X }` where X is a local decl.
                            let source = if let Some(m) = source_module.clone() {
                                // (a)
                                ExportSource::Reexport {
                                    module: m,
                                    original: original.to_string(),
                                }
                            } else if let Some((m, imp_orig)) = imports.get(original) {
                                // (b) — forward the imported binding to its real source.
                                // `import * as ns; export { ns }` needs Namespace,
                                // not Reexport, because `*` isn't a real symbol.
                                if imp_orig == "*" {
                                    ExportSource::Namespace { module: m.clone() }
                                } else {
                                    ExportSource::Reexport {
                                        module: m.clone(),
                                        original: imp_orig.clone(),
                                    }
                                }
                            } else {
                                // (c) — genuinely local.
                                ExportSource::Local
                            };
                            out.named
                                .entry(exposed.to_string())
                                .or_insert(source);
                        }
                    }
                    _ => {
                        collect_file_exports(&inner, bytes, out, imports);
                    }
                }
            }
        }
        "ambient_declaration" => {
            // `declare class X {}`, `declare function f(): void`, `declare
            // module 'foo' { ... }` — recurse so the wrapped declaration
            // gets classified by its own arm.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                collect_file_exports(&inner, bytes, out, imports);
            }
        }
        "internal_module" | "module" => {
            // `namespace X { ... }` or `module 'foo' { ... }`. Recurse
            // into the body; each direct child is itself a decl.
            let body = node
                .child_by_field_name("body")
                .or_else(|| find_named_child(node, &["statement_block"]));
            if let Some(body) = body {
                let mut cursor = body.walk();
                for inner in body.children(&mut cursor) {
                    collect_file_exports(&inner, bytes, out, imports);
                }
            }
        }
        _ => {}
    }
}

/// Extract the `'module'` specifier from an `export ... from '...'` statement.
/// Tree-sitter-typescript exposes it as a `source` field on the export_statement
/// node. The raw text includes the surrounding quotes, which we strip here.
pub(crate) fn extract_export_source_module(node: &Node, bytes: &[u8]) -> Option<String> {
    if let Some(src) = node.child_by_field_name("source") {
        if let Ok(raw) = src.utf8_text(bytes) {
            return Some(strip_quotes(raw));
        }
    }
    None
}

pub(crate) fn strip_quotes(s: &str) -> String {
    s.trim()
        .trim_start_matches('"')
        .trim_end_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('\'')
        .to_string()
}

pub(crate) fn find_named_child<'a>(node: &'a Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for c in node.children(&mut cursor) {
        if kinds.iter().any(|k| *k == c.kind()) {
            return Some(c);
        }
    }
    None
}
