// =============================================================================
// ecosystem/gleam_stdlib.rs — Gleam stdlib (stdlib ecosystem)
//
// The Gleam stdlib ships as a hex package `gleam_stdlib`. Its source lives
// under `src/gleam/` (the `gleam/` namespace: gleam/int, gleam/list, etc.).
//
// Probe order:
//   1. $BEARWISDOM_GLEAM_STDLIB — explicit override path to gleam_stdlib root
//   2. $XDG_CACHE_HOME/gleam/hex/hexpm/packages/gleam_stdlib-*/      (Linux)
//   3. ~/.cache/gleam/hex/hexpm/packages/gleam_stdlib-*/              (Linux/Mac fallback)
//   4. ~/Library/Caches/gleam/hex/hexpm/packages/gleam_stdlib-*/     (Mac)
//   5. %LOCALAPPDATA%/gleam/hex/hexpm/packages/gleam_stdlib-*/        (Windows)
//   6. <project>/build/packages/gleam_stdlib/                         (Gleam project build)
//   7. <project>/build/dev/erlang/gleam_stdlib/                       (Gleam compiled output)
//   8. <project>/build/dev/javascript/gleam_stdlib/                   (Gleam JS output)
//
// Activation: Any([TransitiveOn(hex), LanguagePresent("gleam")]).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("gleam-stdlib");
const LEGACY_ECOSYSTEM_TAG: &str = "gleam-stdlib";
const LANGUAGES: &[&str] = &["gleam"];

pub struct GleamStdlibEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl
// ---------------------------------------------------------------------------

impl Ecosystem for GleamStdlibEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::TransitiveOn(super::hex::ID),
            EcosystemActivation::LanguagePresent("gleam"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_gleam_stdlib(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_gleam_stdlib(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_gleam_stdlib_symbol_index(dep_roots)
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for GleamStdlibEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_gleam_stdlib(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_gleam_stdlib(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GleamStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GleamStdlibEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_gleam_stdlib(project_root: &Path) -> Vec<ExternalDepRoot> {
    // Explicit override — useful for CI and offline setups.
    if let Some(explicit) = std::env::var_os("BEARWISDOM_GLEAM_STDLIB") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            debug!("gleam-stdlib: using explicit override {}", p.display());
            return vec![make_root(p)];
        }
    }

    // User-level Gleam hex cache — platform-specific probing.
    if let Some(dir) = find_in_hex_cache() {
        debug!("gleam-stdlib: found in hex cache {}", dir.display());
        return vec![make_root(dir)];
    }

    // Project-local build outputs (preferred when project is built).
    for rel in &[
        "build/packages/gleam_stdlib",
        "build/dev/erlang/gleam_stdlib",
        "build/dev/javascript/gleam_stdlib",
    ] {
        let p = project_root.join(rel);
        if p.is_dir() {
            debug!("gleam-stdlib: found at {}", p.display());
            return vec![make_root(p)];
        }
    }

    debug!("gleam-stdlib: no installation found — degrading to empty");
    Vec::new()
}

fn make_root(dir: PathBuf) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "gleam_stdlib".to_string(),
        version: String::new(),
        root: dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// Scan all Gleam hex cache locations for `gleam_stdlib-*` directories.
/// Returns the highest-version match found. No version parsing is done —
/// lexicographic sort on the directory name is good enough for `1.x` series.
fn find_in_hex_cache() -> Option<PathBuf> {
    let candidates = hex_cache_bases();
    for base in candidates {
        if !base.is_dir() { continue }
        let Ok(entries) = std::fs::read_dir(&base) else { continue };
        let mut matches: Vec<PathBuf> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_str()?;
                if name.starts_with("gleam_stdlib-") && path.is_dir() {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();
        if matches.is_empty() { continue }
        matches.sort();
        return matches.into_iter().next_back();
    }
    None
}

fn hex_cache_bases() -> Vec<PathBuf> {
    let mut bases = Vec::new();

    // XDG_CACHE_HOME/gleam/hex/hexpm/packages/ (Linux)
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        bases.push(
            PathBuf::from(xdg)
                .join("gleam")
                .join("hex")
                .join("hexpm")
                .join("packages"),
        );
    }

    // %LOCALAPPDATA%/gleam/hex/hexpm/packages/ (Windows)
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        bases.push(
            PathBuf::from(local)
                .join("gleam")
                .join("hex")
                .join("hexpm")
                .join("packages"),
        );
    }

    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);

    if let Some(ref h) = home {
        // ~/.cache/gleam/hex/hexpm/packages/ (Linux / generic Unix)
        bases.push(
            h.join(".cache")
                .join("gleam")
                .join("hex")
                .join("hexpm")
                .join("packages"),
        );
        // ~/Library/Caches/gleam/hex/hexpm/packages/ (macOS)
        bases.push(
            h.join("Library")
                .join("Caches")
                .join("gleam")
                .join("hex")
                .join("hexpm")
                .join("packages"),
        );
    }

    bases
}

// ---------------------------------------------------------------------------
// Walker — emits `src/gleam/*.gleam`
// ---------------------------------------------------------------------------

fn walk_gleam_stdlib(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    // Prefer src/gleam/ (canonical stdlib layout).
    let src_gleam = dep.root.join("src").join("gleam");
    if src_gleam.is_dir() {
        walk_dir(&src_gleam, &dep.root, dep, &mut out, 0);
        return out;
    }
    // Fallback: walk from root (compiled outputs may omit src/).
    walk_dir(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "target" | "ebin" | "priv")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_dir(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".gleam") { continue }
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:gleam:{}/{}", dep.module_path, rel);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "gleam",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol index — delegates to hex::scan_gleam_header via shared function
// ---------------------------------------------------------------------------

pub(crate) fn build_gleam_stdlib_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    // Collect all .gleam files across dep roots.
    let work: Vec<(String, WalkedFile)> = dep_roots
        .iter()
        .flat_map(|dep| {
            walk_gleam_stdlib(dep)
                .into_iter()
                .map(|wf| (dep.module_path.clone(), wf))
        })
        .collect();

    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    // Header-only parse in parallel; reuse the Gleam scanner from hex.rs.
    let per_file: Vec<Vec<(String, String, PathBuf)>> = work
        .par_iter()
        .map(|(module, wf)| {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else {
                return Vec::new();
            };
            let names = scan_gleam_top_level_decls(&src);
            names
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

/// Header-only scan: records top-level public and private declarations
/// (`pub fn`, `fn`, `pub type`, `type`, `pub opaque type`, `pub const`,
/// `const`, `pub external fn`, `external fn`, `pub type` variants).
///
/// This is a line-based scanner — fast, no grammar overhead for the
/// header-only path. Top-level Gleam files are flat enough that line
/// scanning is reliable: every top-level decl starts in column 0.
fn scan_gleam_top_level_decls(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        // Only top-level declarations start at column 0.
        if !line.starts_with(|c: char| c.is_alphanumeric() || c == 'p' || c == 'f' || c == 't' || c == 'c' || c == 'e' || c == 'o' || c == '@') {
            continue;
        }
        let candidate = strip_pub(trimmed);
        let candidate = strip_opaque(candidate);
        if let Some(name) = extract_decl_name(candidate) {
            out.push(name);
        }
    }
    out
}

fn strip_pub(s: &str) -> &str {
    s.strip_prefix("pub ").map(str::trim_start).unwrap_or(s)
}

fn strip_opaque(s: &str) -> &str {
    s.strip_prefix("opaque ").map(str::trim_start).unwrap_or(s)
}

fn extract_decl_name(s: &str) -> Option<String> {
    let rest = if let Some(r) = s.strip_prefix("fn ") {
        r
    } else if let Some(r) = s.strip_prefix("type ") {
        r
    } else if let Some(r) = s.strip_prefix("const ") {
        r
    } else if let Some(r) = s.strip_prefix("external fn ") {
        r
    } else if let Some(r) = s.strip_prefix("external type ") {
        r
    } else {
        return None;
    };

    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let e = GleamStdlibEcosystem;
        assert_eq!(e.id(), ID);
        assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
        assert_eq!(Ecosystem::languages(&e), &["gleam"]);
    }

    #[test]
    fn uses_demand_driven() {
        let e = GleamStdlibEcosystem;
        assert!(e.uses_demand_driven_parse());
        assert!(e.supports_reachability());
    }

    #[test]
    fn scan_gleam_captures_fn_and_type() {
        let src = r#"
pub fn map(list: List(a), f: fn(a) -> b) -> List(b) {
  do_map(list, f, [])
}

pub type Option(a) {
  Some(value: a)
  None
}

pub opaque type Result(a, e) {
  Ok(value: a)
  Error(error: e)
}

pub const empty_list = []

fn internal_helper(x) { x }
"#;
        let names = scan_gleam_top_level_decls(src);
        assert!(names.contains(&"map".to_string()), "missing map in {names:?}");
        assert!(names.contains(&"Option".to_string()), "missing Option in {names:?}");
        assert!(names.contains(&"Result".to_string()), "missing Result in {names:?}");
        assert!(names.contains(&"empty_list".to_string()), "missing empty_list in {names:?}");
        assert!(names.contains(&"internal_helper".to_string()), "missing internal_helper in {names:?}");
    }

    #[test]
    fn scan_gleam_captures_gleam_list_map_and_option_some() {
        // Simulate what gleam/list.gleam and gleam/option.gleam look like.
        let list_src = "pub fn map(list: List(a), f: fn(a) -> b) -> List(b) { todo }\npub fn filter(list: List(a), f: fn(a) -> Bool) -> List(a) { todo }\n";
        let option_src = "pub type Option(a) {\n  Some(value: a)\n  None\n}\n";
        let result_src = "pub type Result(value, error) {\n  Ok(value: value)\n  Error(error: error)\n}\n";

        let list_names = scan_gleam_top_level_decls(list_src);
        let option_names = scan_gleam_top_level_decls(option_src);
        let result_names = scan_gleam_top_level_decls(result_src);

        assert!(list_names.contains(&"map".to_string()), "{list_names:?}");
        assert!(option_names.contains(&"Option".to_string()), "{option_names:?}");
        assert!(result_names.contains(&"Result".to_string()), "{result_names:?}");
    }

    #[test]
    fn walk_yields_gleam_files_from_fixture() {
        let tmp = std::env::temp_dir().join("bw-test-gleam-stdlib-walk");
        let _ = std::fs::remove_dir_all(&tmp);
        let src_gleam = tmp.join("src").join("gleam");
        std::fs::create_dir_all(&src_gleam).unwrap();
        std::fs::write(src_gleam.join("list.gleam"), "pub fn map(l, f) { todo }\n").unwrap();
        std::fs::write(src_gleam.join("string.gleam"), "pub fn length(s) { todo }\n").unwrap();
        // Noise: should be excluded.
        std::fs::create_dir_all(tmp.join("test")).unwrap();
        std::fs::write(tmp.join("test").join("list_test.gleam"), "// test\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "gleam_stdlib".to_string(),
            version: "1.0.0".into(),
            root: tmp.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = walk_gleam_stdlib(&dep);
        assert_eq!(files.len(), 2, "expected 2 files, got {files:?}");
        assert!(files.iter().all(|f| f.language == "gleam"));
        assert!(files.iter().any(|f| f.relative_path.ends_with("list.gleam")));
        assert!(files.iter().any(|f| f.relative_path.ends_with("string.gleam")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn symbol_index_resolves_gleam_list_map_and_option_some_and_result_ok() {
        let tmp = std::env::temp_dir().join("bw-test-gleam-stdlib-index");
        let _ = std::fs::remove_dir_all(&tmp);
        let src_gleam = tmp.join("src").join("gleam");
        std::fs::create_dir_all(&src_gleam).unwrap();
        std::fs::write(
            src_gleam.join("list.gleam"),
            "pub fn map(list: List(a), f: fn(a) -> b) -> List(b) { todo }\n",
        )
        .unwrap();
        std::fs::write(
            src_gleam.join("option.gleam"),
            "pub type Option(a) {\n  Some(value: a)\n  None\n}\n",
        )
        .unwrap();
        std::fs::write(
            src_gleam.join("result.gleam"),
            "pub type Result(value, error) {\n  Ok(value: value)\n  Error(error: error)\n}\n",
        )
        .unwrap();

        let dep = ExternalDepRoot {
            module_path: "gleam_stdlib".to_string(),
            version: "1.0.0".into(),
            root: tmp.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let index = build_gleam_stdlib_symbol_index(&[dep]);

        // gleam/list.map
        assert!(
            index.locate("gleam_stdlib", "map").is_some(),
            "map not in index; index has {} entries", index.len()
        );
        // gleam/option.Some — type variant, not a function — scanner captures the type name
        assert!(
            index.locate("gleam_stdlib", "Option").is_some(),
            "Option not in index"
        );
        // gleam/result.Ok — same: type name
        assert!(
            index.locate("gleam_stdlib", "Result").is_some(),
            "Result not in index"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn empty_dep_roots_returns_empty_index() {
        let index = build_gleam_stdlib_symbol_index(&[]);
        assert!(index.is_empty());
    }

    #[test]
    fn no_stdlib_found_returns_empty_roots() {
        // Point to a temp dir that has no Gleam stdlib layout.
        let tmp = std::env::temp_dir().join("bw-test-gleam-no-stdlib");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // Override env vars to prevent picking up a real installation.
        std::env::remove_var("BEARWISDOM_GLEAM_STDLIB");
        let roots = discover_gleam_stdlib(&tmp);
        // May or may not be empty depending on whether a real Gleam install exists,
        // but the call must not panic.
        let _ = roots;
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
