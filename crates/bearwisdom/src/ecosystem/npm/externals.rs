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

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tracing::debug;

use crate::ecosystem::externals::ExternalDepRoot;
use crate::ecosystem::manifest::npm::NpmManifest;
use crate::ecosystem::manifest::ManifestReader;

use super::walk::resolve_package_entry_path;
use super::{
    collect_bare_reexports_recursive, collect_ts_user_imports, find_node_modules,
    find_node_modules_with_ancestors, is_valid_npm_module_path, node_builtins,
    npm_package_name_from_spec, package_declares_globals, package_ships_scss,
    read_nested_package_json_deps, read_single_package_json_deps, scan_for_scss_bounded,
    LEGACY_ECOSYSTEM_TAG,
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
    let Some(data) = manifest.read(project_root) else {
        return Vec::new();
    };
    if data.dependencies.is_empty() {
        return Vec::new();
    }

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
        if builtins.contains(dep.as_str()) {
            continue;
        }
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
                if primary.is_dir() && package_declares_globals(&primary) {
                    return true;
                }
                if !dep.starts_with("@types/") {
                    if !dep.starts_with('@') {
                        let types_dir = nm.join("@types").join(dep);
                        if types_dir.is_dir() && package_declares_globals(&types_dir) {
                            return true;
                        }
                    } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                        let types_dir = nm.join("@types").join(&escaped);
                        if types_dir.is_dir() && package_declares_globals(&types_dir) {
                            return true;
                        }
                    }
                }
                false
            });
            let any_install_ships_scss = project_has_scss
                && node_modules_roots.iter().any(|nm| {
                    let primary = nm.join(dep);
                    primary.is_dir() && package_ships_scss(&primary)
                });
            if !user_imports_dep
                && !user_imports_companion
                && !any_install_declares_globals
                && !any_install_ships_scss
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
            if primary.is_dir() {
                pkg_roots.push((primary, dep.clone()))
            }
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
        if scan_range.is_empty() {
            break;
        }
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

        if transitive_specs.is_empty() {
            break;
        }
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
            push_transitive_spec_root(package_spec, &probe_roots, &mut seen, &mut roots);
            // Follow the DefinitelyTyped companion too: a transitively reached
            // runtime package can ship without its own `.d.ts` (its `types`
            // field and `index.d.ts` are absent), in which case every member
            // it declares lives in `@types/<pkg>`. That companion is only
            // probed by the direct-dep loop, so without this the transitive
            // package is reached type-less and its members never enter the
            // index. Probe the companion against the SAME roots — for pnpm
            // the companion sits as a sibling symlink in the parent dep's own
            // store `node_modules/`, which `parent_local_nm` already covers.
            if let Some((companion_spec, companion_label)) =
                definitely_typed_companion(package_spec)
            {
                push_transitive_companion_root(
                    &companion_spec,
                    &companion_label,
                    &probe_roots,
                    &mut seen,
                    &mut roots,
                );
            }
        }
    }

    roots
}

/// Push the first existing install of `package_spec` found across
/// `probe_roots` as a transitive dep root, labelled by its package name.
/// Honours the first-writer-wins `seen` dedupe; one canonical install per
/// spec is enough.
fn push_transitive_spec_root(
    package_spec: &str,
    probe_roots: &[&Path],
    seen: &mut std::collections::HashSet<PathBuf>,
    roots: &mut Vec<ExternalDepRoot>,
) {
    for nm_root in probe_roots {
        let candidate = nm_root.join(package_spec);
        if !candidate.is_dir() {
            continue;
        }
        if !seen.insert(candidate.clone()) {
            continue;
        }
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

/// Push the first existing install of a DefinitelyTyped companion directory
/// (`<nm_root>/@types/<escaped>`) as its own dep root, labelled with the
/// `@types/...` module_path so its content stays classified as
/// DefinitelyTyped regardless of which spec led here. Same first-writer-wins
/// `seen` dedupe as the runtime push.
fn push_transitive_companion_root(
    companion_rel: &str,
    companion_label: &str,
    probe_roots: &[&Path],
    seen: &mut std::collections::HashSet<PathBuf>,
    roots: &mut Vec<ExternalDepRoot>,
) {
    for nm_root in probe_roots {
        let candidate = nm_root.join(companion_rel);
        if !candidate.is_dir() {
            continue;
        }
        if !seen.insert(candidate.clone()) {
            continue;
        }
        roots.push(ExternalDepRoot {
            module_path: companion_label.to_string(),
            version: String::from("unknown"),
            root: candidate,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
        break;
    }
}

/// Compute the DefinitelyTyped companion for a package spec: the relative
/// path under a `node_modules/` root (`@types/<escaped>`) and the canonical
/// `@types/...` module_path label. `@types/*` specs have no companion of
/// their own (they ARE the type source). Returns None for those and for any
/// spec that doesn't reduce to a valid module path.
fn definitely_typed_companion(package_spec: &str) -> Option<(String, String)> {
    if package_spec.starts_with("@types/") {
        return None;
    }
    let label = if package_spec.starts_with('@') {
        let escaped = definitely_typed_scoped_name(package_spec)?;
        format!("@types/{escaped}")
    } else {
        format!("@types/{package_spec}")
    };
    if !is_valid_npm_module_path(&label) {
        return None;
    }
    // The on-disk relative path mirrors the label exactly.
    Some((label.clone(), label))
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
    let real_root = std::fs::canonicalize(dep_root)
        .ok()
        .unwrap_or_else(|| dep_root.to_path_buf());
    let parent = real_root.parent()?;
    let parent_name = parent.file_name()?.to_str()?;
    if parent_name.starts_with('@') {
        parent.parent().map(|p| p.to_path_buf())
    } else {
        Some(parent.to_path_buf())
    }
}

// ---------------------------------------------------------------------------
// Discovery — shared small helpers
// ---------------------------------------------------------------------------

/// DefinitelyTyped publishes types for scoped packages at
/// `@types/{scope}__{name}` because npm disallows nested `@` inside a scope
/// path. Returns None for non-scoped names.
pub(crate) fn definitely_typed_scoped_name(dep: &str) -> Option<String> {
    let rest = dep.strip_prefix('@')?;
    let (scope, name) = rest.split_once('/')?;
    if scope.is_empty() || name.is_empty() {
        return None;
    }
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

    if declared.is_empty() {
        return Vec::new();
    }

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
        if builtins.contains(dep.as_str()) {
            continue;
        }
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
                if primary.is_dir() && package_declares_globals(&primary) {
                    return true;
                }
                if !dep.starts_with("@types/") {
                    if !dep.starts_with('@') {
                        let types_dir = nm.join("@types").join(dep);
                        if types_dir.is_dir() && package_declares_globals(&types_dir) {
                            return true;
                        }
                    } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                        let types_dir = nm.join("@types").join(&escaped);
                        if types_dir.is_dir() && package_declares_globals(&types_dir) {
                            return true;
                        }
                    }
                }
                false
            });
            let any_install_ships_scss = project_has_scss
                && node_modules_roots.iter().any(|nm| {
                    let primary = nm.join(dep);
                    primary.is_dir() && package_ships_scss(&primary)
                });
            if !user_imports_dep
                && !user_imports_companion
                && !any_install_declares_globals
                && !any_install_ships_scss
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
            if primary.is_dir() {
                pkg_roots.push((primary, dep.clone()))
            }
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
        if scan_range.is_empty() {
            break;
        }
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

        if transitive_specs.is_empty() {
            break;
        }
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
            push_transitive_spec_root(package_spec, &probe_roots, &mut seen, &mut roots);
            // Mirror the project-level path: follow the DefinitelyTyped
            // companion so a transitively reached but type-less runtime
            // package still surfaces the members declared in `@types/<pkg>`.
            if let Some((companion_spec, companion_label)) =
                definitely_typed_companion(package_spec)
            {
                push_transitive_companion_root(
                    &companion_spec,
                    &companion_label,
                    &probe_roots,
                    &mut seen,
                    &mut roots,
                );
            }
        }
    }

    roots
}

#[cfg(test)]
#[path = "externals_tests.rs"]
mod tests;
