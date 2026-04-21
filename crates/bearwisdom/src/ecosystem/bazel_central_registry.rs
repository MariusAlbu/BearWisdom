// =============================================================================
// ecosystem/bazel_central_registry.rs — Bazel / BCR ecosystem
//
// Covers both bzlmod (MODULE.bazel) and legacy WORKSPACE-based projects.
// Discovers external dep roots from the Bazel output-base external/ directory
// and the project-local bazel-<name>/external/ symlink, then walks .bzl,
// BUILD, and BUILD.bazel files for indexing.
//
// Synthetic symbols are emitted for the Bazel native built-in rules (cc_*,
// java_*, py_*, genrule, …) which are implemented in Java and have no .bzl
// source on disk, using the virtual path `ext:bazel-builtins:rules.bzl`.
//
// Activation: Any([ManifestMatch, LanguagePresent("starlark")]).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

pub const ID: EcosystemId = EcosystemId::new("bazel-central-registry");
const LEGACY_ECOSYSTEM_TAG: &str = "bazel-central-registry";
const LANGUAGES: &[&str] = &["starlark"];

// ---------------------------------------------------------------------------
// Manifest specs
// ---------------------------------------------------------------------------

const MANIFESTS: &[ManifestSpec] = &[
    ManifestSpec {
        glob: "**/MODULE.bazel",
        parse: parse_module_bazel,
    },
    ManifestSpec {
        glob: "**/WORKSPACE{,.bazel}",
        parse: parse_workspace,
    },
];

fn parse_module_bazel(path: &Path) -> std::io::Result<crate::ecosystem::manifest::ManifestData> {
    use crate::ecosystem::manifest::ManifestData;
    let content = std::fs::read_to_string(path)?;
    let deps = extract_bzlmod_deps(&content);
    let mut data = ManifestData::default();
    data.dependencies = deps.into_iter().collect();
    Ok(data)
}

fn parse_workspace(path: &Path) -> std::io::Result<crate::ecosystem::manifest::ManifestData> {
    use crate::ecosystem::manifest::ManifestData;
    let content = std::fs::read_to_string(path)?;
    let deps = extract_workspace_deps(&content);
    let mut data = ManifestData::default();
    data.dependencies = deps.into_iter().collect();
    Ok(data)
}

// ---------------------------------------------------------------------------
// Ecosystem impl
// ---------------------------------------------------------------------------

pub struct BazelCentralRegistryEcosystem;

impl Ecosystem for BazelCentralRegistryEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("starlark"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_bazel_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_bazel_root(dep)
    }

    /// Emit synthetic `ParsedFile` entries for Bazel built-in rules. These
    /// are returned unconditionally (regardless of the dep root path) so
    /// the resolver can close native rule references like `cc_library(...)`.
    fn parse_metadata_only(&self, _dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(vec![synth_builtin_rules()])
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for BazelCentralRegistryEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_bazel_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_bazel_root(dep)
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        Some(vec![synth_builtin_rules()])
    }
}

/// Process-wide shared instance.
pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<BazelCentralRegistryEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(BazelCentralRegistryEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Locate Bazel external dependency roots for a project.
///
/// Two paths are probed in order:
///   1. `<project>/bazel-<dirname>/external/` — project-local output-base
///      symlink that Bazel creates after any build/query.
///   2. `~/.cache/bazel/_bazel_<user>/<hash>/external/` (Linux) or
///      `%USERPROFILE%/_bazel_<user>/<hash>/external/` (Windows) — the
///      real on-disk output-base cache.
///
/// Each subdirectory under `external/` is one dependency. We emit an
/// `ExternalDepRoot` per subdirectory whose name was declared in the
/// project manifests (or all of them if we can't read the manifest).
pub fn discover_bazel_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let declared = declared_dep_names(project_root);

    let mut externals_dirs: Vec<PathBuf> = Vec::new();

    // 1. Project-local bazel-<name>/external/ symlink.
    if let Some(dir_name) = project_root.file_name().and_then(|n| n.to_str()) {
        let local_link = project_root.join(format!("bazel-{dir_name}")).join("external");
        if local_link.is_dir() {
            externals_dirs.push(local_link);
        }
        // Generic `bazel-bin` adjacent fallback — some projects use bazel-<project>.
        let plain_link = project_root.join("bazel-out").parent()
            .map(|p| p.join("external"))
            .filter(|p| p.is_dir());
        if let Some(p) = plain_link { externals_dirs.push(p); }
    }

    // 2. Global output-base cache.
    for cache_ext in find_output_base_externals() {
        externals_dirs.push(cache_ext);
    }

    if externals_dirs.is_empty() {
        debug!("BazelBCR: no external/ directories found for {}", project_root.display());
        return Vec::new();
    }

    let mut roots: Vec<ExternalDepRoot> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for ext_dir in &externals_dirs {
        let Ok(entries) = std::fs::read_dir(ext_dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let Some(dep_name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // Skip Bazel-internal repos prefixed with `_`, `bazel_tools`, etc.
            if dep_name.starts_with('_') || dep_name == "bazel_tools" || dep_name == "local_config_cc" {
                continue;
            }
            if seen.contains(dep_name) { continue; }
            // If we parsed manifests, only include declared deps; otherwise include all.
            if !declared.is_empty() && !declared.contains(dep_name) { continue; }
            seen.insert(dep_name.to_string());
            roots.push(ExternalDepRoot {
                module_path: dep_name.to_string(),
                version: String::new(),
                root: path,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
        }
    }

    debug!("BazelBCR: {} external dep roots", roots.len());
    roots
}

/// Parse all manifests under `project_root` and collect the union of declared
/// dependency names (both bzlmod and WORKSPACE formats).
fn declared_dep_names(project_root: &Path) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();

    // MODULE.bazel
    let module_path = project_root.join("MODULE.bazel");
    if let Ok(content) = std::fs::read_to_string(&module_path) {
        for n in extract_bzlmod_deps(&content) { names.insert(n); }
    }

    // WORKSPACE / WORKSPACE.bazel
    for candidate in ["WORKSPACE", "WORKSPACE.bazel"] {
        let ws_path = project_root.join(candidate);
        if let Ok(content) = std::fs::read_to_string(&ws_path) {
            for n in extract_workspace_deps(&content) { names.insert(n); }
            break;
        }
    }

    names
}

/// Find `external/` directories inside the Bazel output-base cache.
///
/// Layout: `~/.cache/bazel/_bazel_<user>/<hash>/external/` (Linux/macOS)
///          `%USERPROFILE%/_bazel_<user>/<hash>/external/` (Windows)
fn find_output_base_externals() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let home = if cfg!(windows) {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    } else {
        // ~/.cache/bazel on Linux; ~/Library/Caches/bazel on macOS is also common.
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache").join("bazel"))
    };
    let Some(cache_root) = home else { return found };

    // On Linux/macOS: $HOME/.cache/bazel/_bazel_<user>/
    // The actual directory is $HOME/.cache/bazel/_bazel_<user>/<hash>/external/
    // We walk two levels deep to find any hash directory.
    let bazel_dir = if cfg!(windows) { cache_root.join(".bazel") } else { cache_root };
    if !bazel_dir.is_dir() { return found; }

    let Ok(user_dirs) = std::fs::read_dir(&bazel_dir) else { return found };
    for user_entry in user_dirs.flatten() {
        let user_path = user_entry.path();
        if !user_path.is_dir() { continue; }
        let Ok(hash_dirs) = std::fs::read_dir(&user_path) else { continue };
        for hash_entry in hash_dirs.flatten() {
            let ext = hash_entry.path().join("external");
            if ext.is_dir() {
                found.push(ext);
            }
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

const MAX_WALK_DEPTH: u32 = 8;

pub fn walk_bazel_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | "node_modules" | "__pycache__") || name.starts_with('.') {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            if !is_bazel_source_file(&path) { continue; }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:bazel:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "starlark",
            });
        }
    }
}

fn is_bazel_source_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    matches!(name, "BUILD" | "BUILD.bazel") || name.ends_with(".bzl")
}

// ---------------------------------------------------------------------------
// Manifest parsing — line-regex MVP
// ---------------------------------------------------------------------------

/// Extract `bazel_dep(name = "...", ...)` entries from a MODULE.bazel file.
pub fn extract_bzlmod_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("bazel_dep(") { continue; }
        if let Some(name) = extract_kwarg(trimmed, "name") {
            if !name.is_empty() { deps.push(name); }
        }
    }
    deps
}

/// Extract `http_archive(name = "...")` and `git_repository(name = "...")`
/// entries from a legacy WORKSPACE file.
pub fn extract_workspace_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_block = false;
    let mut block_buf = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if !in_block {
            if trimmed.starts_with("http_archive(")
                || trimmed.starts_with("git_repository(")
                || trimmed.starts_with("new_git_repository(")
                || trimmed.starts_with("http_file(")
            {
                in_block = true;
                block_buf.clear();
                block_buf.push_str(trimmed);
                block_buf.push('\n');
                if trimmed.ends_with(')') {
                    if let Some(name) = extract_kwarg(&block_buf, "name") {
                        if !name.is_empty() { deps.push(name); }
                    }
                    in_block = false;
                }
            }
        } else {
            block_buf.push_str(trimmed);
            block_buf.push('\n');
            if trimmed == ")" || trimmed.ends_with(')') {
                if let Some(name) = extract_kwarg(&block_buf, "name") {
                    if !name.is_empty() { deps.push(name); }
                }
                in_block = false;
            }
        }
    }
    deps
}

/// Extract `key = "value"` from a Starlark-ish single-line or buffered block.
fn extract_kwarg(text: &str, key: &str) -> Option<String> {
    let needle = format!("name = \"");
    // Only match the right key= form.
    let search = format!("{key} = \"");
    let start = text.find(&search)?;
    let rest = &text[start + search.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ---------------------------------------------------------------------------
// Synthetic built-in rules
// ---------------------------------------------------------------------------

/// The Bazel native built-in rules. These are implemented in Java and have no
/// .bzl source. We emit a single synthetic `ParsedFile` at the virtual path
/// `ext:bazel-builtins:rules.bzl` so that BUILD-file references like
/// `cc_library(...)` resolve to a real symbol instead of an unresolved ref.
const BUILTIN_RULES: &[&str] = &[
    "cc_library",
    "cc_binary",
    "cc_test",
    "java_library",
    "java_binary",
    "java_test",
    "py_library",
    "py_binary",
    "py_test",
    "genrule",
    "filegroup",
    "exports_files",
    "package",
    "alias",
    "config_setting",
    "constraint_value",
    "platform",
    "toolchain",
    "sh_library",
    "sh_binary",
    "sh_test",
    "proto_library",
    "test_suite",
];

pub fn synth_builtin_rules() -> ParsedFile {
    let virtual_path = "ext:bazel-builtins:rules.bzl".to_string();
    let symbols: Vec<ExtractedSymbol> = BUILTIN_RULES
        .iter()
        .enumerate()
        .map(|(i, &rule)| ExtractedSymbol {
            name: rule.to_string(),
            qualified_name: rule.to_string(),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: i as u32,
            end_line: i as u32,
            start_col: 0,
            end_col: 0,
            signature: Some(format!("def {rule}(**kwargs)")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        })
        .collect();

    let sym_count = symbols.len();
    ParsedFile {
        path: virtual_path,
        language: "starlark".to_string(),
        content_hash: format!("bazel-builtins-{sym_count}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let eco = BazelCentralRegistryEcosystem;
        assert_eq!(eco.id(), ID);
        assert_eq!(Ecosystem::kind(&eco), EcosystemKind::Package);
        assert_eq!(Ecosystem::languages(&eco), &["starlark"]);
        assert_eq!(eco.id().as_str(), "bazel-central-registry");
    }

    #[test]
    fn parse_module_bazel_extracts_bazel_deps() {
        let content = r#"
module(
    name = "bazel_skylib",
    version = "1.9.0",
    compatibility_level = 1,
)

bazel_dep(name = "platforms", version = "0.0.10")
bazel_dep(name = "rules_license", version = "1.0.0")
bazel_dep(name = "stardoc", version = "0.8.0", dev_dependency = True, repo_name = "io_bazel_stardoc")
bazel_dep(name = "rules_cc", version = "0.0.17", dev_dependency = True)
"#;
        let deps = extract_bzlmod_deps(content);
        assert!(deps.contains(&"platforms".to_string()), "platforms missing");
        assert!(deps.contains(&"rules_license".to_string()), "rules_license missing");
        assert!(deps.contains(&"stardoc".to_string()), "stardoc missing");
        assert!(deps.contains(&"rules_cc".to_string()), "rules_cc missing");
        // module() itself is not a dep.
        assert!(!deps.contains(&"bazel_skylib".to_string()), "module name should not be a dep");
    }

    #[test]
    fn parse_workspace_extracts_http_archive_deps() {
        let content = r#"
workspace(name = "bazel_skylib")

http_archive(
    name = "rules_cc",
    sha256 = "abc605dd850f813bb37004b77db20106a19311a96b2da1c92b789da529d28fe1",
    strip_prefix = "rules_cc-0.0.17",
    urls = ["https://github.com/bazelbuild/rules_cc/releases/download/0.0.17/rules_cc-0.0.17.tar.gz"],
)

http_archive(
    name = "rules_shell",
    sha256 = "d8cd4a3a91fc1dc68d4c7d6b655f09def109f7186437e3f50a9b60ab436a0c53",
    url = "https://github.com/bazelbuild/rules_shell/releases/download/v0.3.0/rules_shell-v0.3.0.tar.gz",
)
"#;
        let deps = extract_workspace_deps(content);
        assert!(deps.contains(&"rules_cc".to_string()), "rules_cc missing from WORKSPACE");
        assert!(deps.contains(&"rules_shell".to_string()), "rules_shell missing from WORKSPACE");
    }

    #[test]
    fn builtin_rules_contains_cc_library() {
        let pf = synth_builtin_rules();
        assert_eq!(pf.path, "ext:bazel-builtins:rules.bzl");
        assert_eq!(pf.language, "starlark");
        let has_cc = pf.symbols.iter().any(|s| s.name == "cc_library");
        assert!(has_cc, "cc_library not in builtin rules");
        let has_genrule = pf.symbols.iter().any(|s| s.name == "genrule");
        assert!(has_genrule, "genrule not in builtin rules");
        assert_eq!(pf.symbols.len(), BUILTIN_RULES.len());
    }

    #[test]
    fn builtin_rule_count() {
        // Keep in sync with the BUILTIN_RULES constant.
        assert_eq!(BUILTIN_RULES.len(), 23);
    }

    #[test]
    fn walk_bazel_root_returns_starlark_files() {
        let tmp = std::env::temp_dir().join("bw-test-bazel-walk");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("lib")).unwrap();
        std::fs::write(tmp.join("lib").join("paths.bzl"), "def join(*args): pass").unwrap();
        std::fs::write(tmp.join("BUILD"), "filegroup(name = \"all\")").unwrap();
        std::fs::write(tmp.join("not_starlark.py"), "x = 1").unwrap();

        let dep = ExternalDepRoot {
            module_path: "test_dep".to_string(),
            version: String::new(),
            root: tmp.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let files = walk_bazel_root(&dep);
        assert_eq!(files.len(), 2, "expected BUILD + paths.bzl, got {}", files.len());
        assert!(files.iter().all(|f| f.language == "starlark"));
        assert!(files.iter().any(|f| f.relative_path.ends_with("paths.bzl")));
        assert!(files.iter().any(|f| f.relative_path.ends_with("BUILD")));
        // .py files must not appear.
        assert!(files.iter().all(|f| !f.relative_path.ends_with(".py")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn legacy_locator_tag() {
        assert_eq!(
            ExternalSourceLocator::ecosystem(&BazelCentralRegistryEcosystem),
            "bazel-central-registry"
        );
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
