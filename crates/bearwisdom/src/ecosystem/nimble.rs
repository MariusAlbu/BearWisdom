// =============================================================================
// ecosystem/nimble.rs — Nimble ecosystem (Nim)
//
// Phase 2 + 3: consolidates `indexer/externals/nim.rs`. No separate
// manifest reader — .nimble file parsing lives here.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("nimble");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["nim"];
const LEGACY_ECOSYSTEM_TAG: &str = "nim";

pub struct NimbleEcosystem;

impl Ecosystem for NimbleEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] {
        // Nim packages use `<pkg>.nimble` (extension match).
        &[(".nimble", "nim")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["nimcache"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `*.nimble` OR raw `.nim` files. The Nim stdlib
        // is part of every Nim toolchain (probed from the compiler's lib/
        // directory in `find_nim_stdlib`), so any project with `.nim` files
        // benefits from walking it. ManifestMatch alone misses bare .nim
        // directories AND fails when the .nimble file isn't picked up by
        // the manifest scanner.
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("nim"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_nim_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_nim_root(dep)
    }

    fn supports_reachability(&self) -> bool { true }
    fn resolve_import(
        &self, dep: &ExternalDepRoot, _p: &str, _s: &[&str],
    ) -> Vec<WalkedFile> { walk_nim_narrowed(dep) }
    fn resolve_symbol(
        &self, dep: &ExternalDepRoot, _f: &str,
    ) -> Vec<WalkedFile> { walk_nim_narrowed(dep) }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_nim_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn demand_pre_pull(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> Vec<WalkedFile> {
        nim_stdlib_pre_pull(dep_roots)
    }
}

impl ExternalSourceLocator for NimbleEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_nim_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_nim_root(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<NimbleEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NimbleEcosystem)).clone()
}

// ===========================================================================
// Manifest reader
// ===========================================================================

/// Surfaces `*.nimble` `requires` declarations in
/// `ProjectContext.manifests[ManifestKind::Nimble]`.
pub struct NimbleManifest;

impl crate::ecosystem::manifest::ManifestReader for NimbleManifest {
    fn kind(&self) -> crate::ecosystem::manifest::ManifestKind {
        crate::ecosystem::manifest::ManifestKind::Nimble
    }

    fn read(&self, project_root: &Path) -> Option<crate::ecosystem::manifest::ManifestData> {
        let deps = parse_nimble_requires(project_root);
        if deps.is_empty() { return None }
        let mut data = crate::ecosystem::manifest::ManifestData::default();
        data.dependencies = deps.into_iter().collect();
        Some(data)
    }
}

// ===========================================================================
// Discovery
// ===========================================================================

pub fn discover_nim_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = parse_nimble_requires(project_root);
    let user_imports: Vec<String> = collect_nim_user_imports(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();

    // Nimble package roots — `~/.nimble/pkgs2/<dep>-<version>-<hash>/`.
    if let Some(pkgs_dir) = find_nimble_pkgs_dir() {
        if let Ok(entries) = std::fs::read_dir(&pkgs_dir) {
            let all_entries: Vec<_> = entries.flatten().collect();
            for dep_name in &declared {
                let prefix = format!("{dep_name}-");
                let mut matches: Vec<PathBuf> = all_entries
                    .iter()
                    .filter(|e| {
                        let n = e.file_name();
                        let s = n.to_string_lossy();
                        s.starts_with(&prefix) && e.path().is_dir()
                    })
                    .map(|e| e.path())
                    .collect();
                matches.sort();
                if let Some(best) = matches.pop() {
                    let version = best
                        .file_name().and_then(|n| n.to_str())
                        .and_then(|n| n.strip_prefix(&prefix))
                        .unwrap_or("").to_string();
                    roots.push(ExternalDepRoot {
                        module_path: dep_name.clone(),
                        version,
                        root: best,
                        ecosystem: LEGACY_ECOSYSTEM_TAG,
                        package_id: None,
                        requested_imports: user_imports.clone(),
                    });
                }
            }
        }
    }

    // Nim stdlib root — `<nim-install>/lib/`. Imports of `std/sequtils`,
    // `strutils`, `tables`, etc. resolve here. The compiler's lib dir is the
    // canonical source for stdlib modules and ships with every Nim install,
    // so adding it as an implicit root covers every Nim project unconditionally.
    if let Some(stdlib_root) = find_nim_stdlib() {
        roots.push(ExternalDepRoot {
            module_path: "nim-stdlib".to_string(),
            version: String::new(),
            root: stdlib_root,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: user_imports.clone(),
        });
    }

    debug!("Nim: {} external roots (Nimble + stdlib)", roots.len());
    roots
}

/// Locate the Nim compiler's lib/ directory. Probes:
///   1. `BEARWISDOM_NIM_STDLIB` — explicit override
///   2. `NIM_HOME` / `NIMHOME` env hints (`<home>/lib`)
///   3. `nim dump` — asks the compiler for its lib path
fn find_nim_stdlib() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_NIM_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() { return Some(p); }
    }
    for env_key in ["NIM_HOME", "NIMHOME"] {
        if let Some(home) = std::env::var_os(env_key) {
            let lib = PathBuf::from(home).join("lib");
            if lib.is_dir() { return Some(lib); }
        }
    }

    // Ask the compiler for its install paths. `nim dump dummy.nim` prints
    // every search path on its own line — `<install>/lib/pure`,
    // `<install>/lib/core`, `<install>/lib/posix`, etc. The first match
    // whose final segment is a known stdlib subdir gives us the install's
    // `lib/` parent in one walk-up.
    use std::process::Command;
    let probe = |program: &str| -> Option<PathBuf> {
        let out = Command::new(program)
            .args(["dump", "dummy.nim"])
            .output()
            .ok()?;
        // `nim dump` writes config to stderr on most versions; merge both
        // streams to be robust.
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for line in combined.lines() {
            let raw = line.trim();
            if raw.is_empty() { continue }
            // Skip log/config noise — only consider lines that look like
            // absolute paths (Windows drive letter or POSIX root).
            let looks_absolute = raw
                .chars()
                .nth(1)
                .map(|c| c == ':')
                .unwrap_or(false)
                || raw.starts_with('/');
            if !looks_absolute { continue }
            let candidate = PathBuf::from(raw);
            // Walk up from `<install>/lib/<subdir>` to `<install>/lib`.
            // Only accept a parent named `lib` so deeper paths
            // (`lib/wrappers/linenoise`) get pulled in too.
            let mut walk = Some(candidate.as_path());
            while let Some(p) = walk {
                if p.file_name().and_then(|n| n.to_str()) == Some("lib") && p.is_dir() {
                    return Some(p.to_path_buf());
                }
                walk = p.parent();
            }
        }
        None
    };
    if let Some(p) = probe("nim") { return Some(p); }
    // Windows shims are `.bat` files; std::process::Command doesn't apply
    // PATHEXT so try the explicit name.
    #[cfg(windows)]
    {
        if let Some(p) = probe("nim.bat") { return Some(p); }
    }

    None
}

// R3 — `import strutils` / `import std/strutils` / `import foo/[bar, baz]`
// scanner + narrowed walk. Stored as the leaf module name plus the dotted
// path; narrowing maps both to file tails (`strutils.nim` / `foo/bar.nim`).

fn collect_nim_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_nim_imports(project_root, &mut out, 0);
    out
}

fn scan_nim_imports(dir: &Path, out: &mut std::collections::HashSet<String>, depth: usize) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | "nimcache" | "tests" | "test" | "examples")
                    || name.starts_with('.') { continue }
            }
            scan_nim_imports(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".nim") { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_nim_imports(&content, out);
        }
    }
}

fn extract_nim_imports(content: &str, out: &mut std::collections::HashSet<String>) {
    for raw in content.lines() {
        let line = raw.trim();
        let rest = match line
            .strip_prefix("import ")
            .or_else(|| line.strip_prefix("from "))
        {
            Some(r) => r,
            None => continue,
        };
        // Strip trailing `; comment` pieces.
        let rest = rest.split('#').next().unwrap_or("").trim();
        // `from x import y` — only the `x` part matters.
        let rest = rest.split(" import ").next().unwrap_or("").trim();
        // Group form: `import foo/[a, b, c]`
        if let Some(open) = rest.find('[') {
            if let Some(close) = rest.find(']') {
                let prefix = rest[..open].trim_end_matches('/').trim();
                let inner = &rest[open + 1..close];
                for sel in inner.split(',') {
                    let sel = sel.trim();
                    if sel.is_empty() { continue }
                    out.insert(if prefix.is_empty() { sel.to_string() } else { format!("{prefix}/{sel}") });
                }
                continue;
            }
        }
        // Comma-separated: `import foo, bar`
        for part in rest.split(',') {
            let part = part.trim();
            if part.is_empty() { continue }
            // `foo as F` → drop alias
            let head = part.split(" as ").next().unwrap_or("").trim();
            if head.is_empty() { continue }
            out.insert(head.to_string());
        }
    }
}

fn nim_module_to_path_tail(module: &str) -> Option<String> {
    let cleaned = module.trim();
    if cleaned.is_empty() { return None }
    // `std/strutils` / `pkg/foo` / `foo` → all map to file paths.
    Some(format!("{}.nim", cleaned.replace('.', "/")))
}

/// Return every plausible file tail for a Nim import. The `std/X` prefix is
/// import-syntax only; on disk the stdlib lives flat under `pure/<X>.nim`,
/// `core/<X>.nim`, `pure/collections/<X>.nim`, etc. Always also offer the
/// leaf basename so the narrowed walker matches regardless of the directory
/// the compiler organises stdlib modules into.
fn nim_module_path_tails(module: &str) -> Vec<String> {
    let mut out = Vec::new();
    let cleaned = module.trim();
    if cleaned.is_empty() { return out; }
    let primary = cleaned.replace('.', "/");
    out.push(format!("{primary}.nim"));
    // `std/strutils` → leaf "strutils.nim"
    if let Some(leaf) = primary.rsplit('/').next() {
        if leaf != primary {
            out.push(format!("{leaf}.nim"));
        }
    }
    // `std/X` is the qualified-stdlib syntax; the file is at `pure/X.nim`
    // and friends — emit the prefix-stripped form too.
    if let Some(rest) = primary.strip_prefix("std/") {
        out.push(format!("{rest}.nim"));
    }
    out
}

fn walk_nim_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() { return walk_nim_root(dep); }
    let tails: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .flat_map(|m| nim_module_path_tails(m))
        .collect();
    if tails.is_empty() { return walk_nim_root(dep); }

    let mut out = Vec::new();
    walk_nim_narrowed_dir(&dep.root, &dep.root, dep, &tails, &mut out, 0);
    out
}

fn walk_nim_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    tails: &std::collections::HashSet<String>,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut dir_files: Vec<(PathBuf, String)> = Vec::new();
    let mut any_match = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "examples" | "docs" | "nimcache") || name.starts_with('.') { continue }
            }
            subdirs.push(path);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".nim") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if tails.iter().any(|t| rel_sub.ends_with(t)) { any_match = true; }
            dir_files.push((path, rel_sub));
        }
    }

    if any_match {
        for (path, rel_sub) in dir_files {
            out.push(WalkedFile {
                relative_path: format!("ext:nim:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "nim",
            });
        }
    }
    for sub in subdirs {
        walk_nim_narrowed_dir(&sub, root, dep, tails, out, depth + 1);
    }
}

pub fn parse_nimble_requires(project_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(project_root) else { return Vec::new() };
    let nimble_file = entries
        .flatten()
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("nimble"));
    let Some(entry) = nimble_file else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(entry.path()) else { return Vec::new() };

    let mut deps = Vec::new();
    let mut record_dep = |raw: &str, deps: &mut Vec<String>| {
        let dep = raw.trim();
        if dep.is_empty() { return }
        // Skip https:// URLs — they're not simple package names.
        if dep.starts_with("https://") || dep.starts_with("http://") { return }
        let name = dep
            .split(|c: char| c == '>' || c == '<' || c == '=' || c == '#' || c == '@' || c.is_whitespace())
            .next().unwrap_or("").trim();
        if !name.is_empty() && name != "nim"
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !deps.iter().any(|d| d == name)
        {
            deps.push(name.to_string());
        }
    };

    // Nimble `requires` can appear in several forms:
    //
    //   requires "foo >= 1.0"
    //
    //   requires "foo >= 1.0", \
    //     "bar"
    //
    //   requires "foo",
    //     "bar",                          ← comma-continuation, no parens
    //     "baz"
    //
    //   requires(
    //     "foo",
    //     "bar",
    //   )
    //
    // The continuation lines have no `requires` keyword; we detect them by
    // tracking whether the previous requires-carrying line ended with `,` or
    // `\` (implicit continuation), or whether we entered a `(` block.
    let mut in_requires = false; // comma-continuation mode (no parens)
    let mut in_block = false;    // explicit `requires(` block
    for line in content.lines() {
        let trimmed = line.trim();

        // Single-line `requires "name"` and multi-line `requires(` opener.
        if trimmed.starts_with("requires") {
            // `requires(` form — block extends until the matching `)`.
            if trimmed.contains('(') && !trimmed.contains(')') {
                in_block = true;
                in_requires = false;
            }
            // Strip the keyword + opening paren before scanning quoted args.
            let after_kw = trimmed.trim_start_matches("requires").trim_start_matches('(');
            for part in after_kw.split('"') {
                record_dep(part, &mut deps);
            }
            // Comma-continuation: the line (or its visible content after
            // stripping comments) ends with `,` or `\` — subsequent
            // indented lines belong to the same requires statement.
            let visible = after_kw.split('#').next().unwrap_or("").trim_end_matches('\\').trim();
            in_requires = !in_block && visible.ends_with(',');
            continue;
        }

        // Inside a `requires(...)` block.
        if in_block {
            if trimmed.contains(')') {
                in_block = false;
                in_requires = false;
            }
            for part in trimmed.split('"') {
                record_dep(part, &mut deps);
            }
            continue;
        }

        // Comma-continuation lines: indented lines containing quoted dep
        // strings that follow a `requires` line ending with `,`.
        if in_requires {
            // A line that is not indented and is not a continuation (no leading
            // whitespace and not just a comma or quote) ends the block.
            if !trimmed.is_empty() && !line.starts_with(char::is_whitespace)
                && !trimmed.starts_with('"') && !trimmed.starts_with(',')
            {
                in_requires = false;
                continue;
            }
            for part in trimmed.split('"') {
                record_dep(part, &mut deps);
            }
            // Keep continuation mode while the line ends with `,`.
            let visible = trimmed.split('#').next().unwrap_or("").trim_end_matches('\\').trim();
            if !visible.ends_with(',') {
                in_requires = false;
            }
        }
    }
    deps
}

fn find_nimble_pkgs_dir() -> Option<PathBuf> {
    if let Ok(nimble_dir) = std::env::var("NIMBLE_DIR") {
        let p = PathBuf::from(&nimble_dir).join("pkgs2");
        if p.is_dir() { return Some(p) }
        let p = PathBuf::from(nimble_dir).join("pkgs");
        if p.is_dir() { return Some(p) }
    }
    let home = dirs::home_dir()?;
    let pkgs2 = home.join(".nimble").join("pkgs2");
    if pkgs2.is_dir() { return Some(pkgs2) }
    let pkgs = home.join(".nimble").join("pkgs");
    if pkgs.is_dir() { return Some(pkgs) }
    None
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_nim_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "tests" | "test" | "examples" | "docs" | "nimcache")
                    || name.starts_with('.')
                { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".nim") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:nim:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "nim",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Stdlib pre-pull (demand-driven pipeline)
// ---------------------------------------------------------------------------

/// Subdirectories of the Nim stdlib pre-pulled unconditionally for every
/// Nim project. These contain the modules most commonly imported (`strutils`,
/// `sequtils`, `tables`, `math`, `os`, `json`, …). Relative to `<lib>/`.
const STDLIB_PRE_PULL_SUBDIRS: &[&str] = &[
    "system",
    "pure",
    "core",
    "std",
];

/// Walk the stdlib subdirs and the main entry file of every nimble package dep,
/// returning WalkedFiles for eager parsing. Called by the demand-driven pipeline
/// before symbol-index queries begin so that stdlib AND package symbols are
/// available on the first resolve pass — before the chain-walker expand loop runs.
///
/// For the stdlib dep: walks `system.nim` + `pure/`, `core/`, `std/` subdirs.
/// For each nimble package dep: includes `<module_path>.nim` at the package root
/// (the canonical entry point for single-file packages like `results.nim`) so
/// bare call targets (`ok`, `tryGet`, `some`, …) resolve on pass 1.
fn nim_stdlib_pre_pull(dep_roots: &[ExternalDepRoot]) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    for dep in dep_roots {
        let root = &dep.root;
        if dep.module_path == "nim-stdlib" {
            // Top-level system.nim is the auto-imported prelude.
            let sys = root.join("system.nim");
            if sys.is_file() {
                let rel = format!("ext:nim:{}/system.nim", dep.module_path);
                out.push(WalkedFile { relative_path: rel, absolute_path: sys, language: "nim" });
            }
            for sub in STDLIB_PRE_PULL_SUBDIRS {
                let dir = root.join(sub);
                if dir.is_dir() {
                    walk_dir_bounded(&dir, root, dep, &mut out, 0);
                }
            }
        } else {
            // For nimble package deps, pre-pull all `.nim` files in the
            // package. Packages like `chronos` expose their surface via
            // re-exported submodules; indexing only the root entry file
            // misses the symbols in `asyncloop.nim`, `asyncsync.nim`, etc.
            // The walker respects the standard exclusions (tests/, examples/,
            // nimcache) so depth is bounded in practice.
            walk_dir_bounded(root, root, dep, &mut out, 0);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------
//
// Nim has no tree-sitter grammar in this crate, so we do a line-based scan
// for top-level declarations. Matches `proc`, `func`, `method`, `iterator`,
// `converter`, `template`, `macro`, `type`, `const`, `var`, `let` followed
// by an identifier at column 0 (or after `export` marker `*`).

fn build_nim_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_nim_root(dep) {
            // For the stdlib root, key by file stem so locate("sequtils",
            // "toSeq") matches the entry built from `pure/collections/sequtils.nim`.
            // For Nimble package roots, key by dep.module_path (package name).
            let module_key = if dep.module_path == "nim-stdlib" {
                wf.absolute_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| dep.module_path.clone())
            } else {
                dep.module_path.clone()
            };
            work.push((module_key, wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            scan_nim_header(&src)
                .into_iter()
                .map(|name| (module.clone(), name, wf.absolute_path.clone()))
                .collect()
        })
        .collect();
    let mut index = SymbolLocationIndex::new();
    for batch in per_file {
        for (module, name, file) in batch {
            index.insert(module, name, file);
        }
    }
    index
}

/// Line-based scan of a Nim source file for top-level declarations. Looks
/// for `proc name*(...)`, `type Foo*`, `const X*`, `var Y*`, etc. — only at
/// column 0 so nested definitions inside proc bodies don't leak through.
pub(crate) fn scan_nim_header(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        // Only consider lines that start at column 0 (no leading whitespace).
        if line.is_empty() || line.starts_with(char::is_whitespace) { continue }
        let kw = match next_nim_keyword(line) {
            Some(kw) => kw,
            None => continue,
        };
        let rest = line[kw.len()..].trim_start();
        let name = extract_nim_identifier(rest);
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

fn next_nim_keyword(line: &str) -> Option<&'static str> {
    for kw in &[
        "proc", "func", "method", "iterator", "converter", "template",
        "macro", "type", "const", "var", "let",
    ] {
        if line.starts_with(kw) {
            // Must be followed by whitespace or `*` to avoid matching
            // identifiers that start with these prefixes.
            let rest = &line[kw.len()..];
            if rest.starts_with(char::is_whitespace)
                || rest.starts_with('*')
                || rest.starts_with('(')
            {
                return Some(kw);
            }
        }
    }
    None
}

/// Grab the identifier at the start of `rest`. Nim identifiers are
/// alphanumeric + underscore; the first char must be alphabetic.
fn extract_nim_identifier(rest: &str) -> String {
    let mut chars = rest.chars();
    let first = match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => c,
        _ => return String::new(),
    };
    let mut name = String::new();
    name.push(first);
    for c in chars {
        if c.is_alphanumeric() || c == '_' { name.push(c) } else { break }
    }
    name
}

#[cfg(test)]
#[path = "nimble_tests.rs"]
mod tests;

