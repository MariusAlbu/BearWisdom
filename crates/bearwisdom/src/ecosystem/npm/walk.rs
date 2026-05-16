// =============================================================================
// ecosystem/npm/walk.rs — file collection / entry resolution / reexport expansion
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use tracing::debug;

use crate::ecosystem::externals::{ExternalDepRoot, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

use super::{is_valid_npm_module_path, normalize_virtual_rel, package_ships_scss, scan_for_scss_bounded};

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

pub(super) const REEXPORT_MAX_DEPTH: u32 = 3;

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

