// =============================================================================
// ecosystem/dart_sdk.rs — Dart SDK stdlib ecosystem
//
// Probes the Dart SDK lib/ directory and indexes the core stdlib packages:
// core, async, collection, convert, io, isolate, math, typed_data,
// developer, ffi.
//
// Probe order:
//   1. BEARWISDOM_DART_SDK env var override
//   2. DART_SDK env var
//   3. FLUTTER_ROOT/bin/cache/dart-sdk/
//   4. `dart` binary on PATH → walk up to find sdk root
//   5. Well-known install paths (Windows: Program Files/Dart/dart-sdk,
//      macOS: /usr/lib/dart, Linux: /usr/lib/dart)
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("dart-sdk");
const LEGACY_ECOSYSTEM_TAG: &str = "dart-sdk";
const LANGUAGES: &[&str] = &["dart"];

/// Sub-libraries of `lib/` that constitute the public Dart SDK stdlib.
const DART_SDK_LIBS: &[&str] = &[
    "core",
    "async",
    "collection",
    "convert",
    "io",
    "isolate",
    "math",
    "typed_data",
    "developer",
    "ffi",
];

pub struct DartSdkEcosystem;

impl Ecosystem for DartSdkEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("dart")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_dart_sdk()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_sdk(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_dart_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for DartSdkEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_dart_sdk()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_dart_sdk(dep)
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn discover_dart_sdk() -> Vec<ExternalDepRoot> {
    let Some(lib_dir) = probe_dart_sdk_lib() else {
        debug!("dart-sdk: no SDK probe succeeded");
        return Vec::new();
    };
    debug!("dart-sdk: using {}", lib_dir.display());
    vec![ExternalDepRoot {
        module_path: "dart-sdk".to_string(),
        version: String::new(),
        root: lib_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_dart_sdk_lib() -> Option<PathBuf> {
    // 1. Explicit override
    if let Some(raw) = std::env::var_os("BEARWISDOM_DART_SDK") {
        let p = PathBuf::from(raw).join("lib");
        if p.is_dir() { return Some(p); }
    }

    // 2. DART_SDK env var (points to sdk root, not lib/)
    if let Some(raw) = std::env::var_os("DART_SDK") {
        let p = PathBuf::from(raw).join("lib");
        if p.is_dir() { return Some(p); }
    }

    // 3. FLUTTER_ROOT bundled dart-sdk
    if let Some(raw) = std::env::var_os("FLUTTER_ROOT") {
        let p = PathBuf::from(raw)
            .join("bin")
            .join("cache")
            .join("dart-sdk")
            .join("lib");
        if p.is_dir() { return Some(p); }
    }

    // 4. `dart` binary on PATH → resolve symlinks and walk up
    if let Some(sdk_root) = dart_bin_sdk_root("dart") {
        let p = sdk_root.join("lib");
        if p.is_dir() { return Some(p); }
    }

    // 5. Well-known install paths
    for candidate in well_known_dart_sdk_paths() {
        let p = candidate.join("lib");
        if p.is_dir() { return Some(p); }
    }

    None
}

/// Invoke `dart --print-sdk-directory` or locate via PATH to find sdk root.
/// The dart binary lives at `<sdk>/bin/dart`.
fn dart_bin_sdk_root(bin: &str) -> Option<PathBuf> {
    // Try `dart --print-sdk-directory` first (Dart 2.x+)
    if let Ok(output) = Command::new(bin).arg("--print-sdk-directory").output() {
        if output.status.success() {
            let s = String::from_utf8(output.stdout).ok()?;
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                let p = PathBuf::from(trimmed);
                if p.is_dir() { return Some(p); }
            }
        }
    }

    // Fallback: locate the binary and walk up
    let which_cmd = if cfg!(windows) { "where" } else { "which" };
    let Ok(output) = Command::new(which_cmd).arg(bin).output() else {
        return None;
    };
    if !output.status.success() { return None; }
    let s = String::from_utf8(output.stdout).ok()?;
    let binary_path = PathBuf::from(s.lines().next()?.trim());
    let resolved = binary_path.canonicalize().unwrap_or(binary_path);
    // <sdk>/bin/dart → parent = <sdk>/bin → parent = <sdk>
    resolved.parent()?.parent().map(|p| p.to_path_buf())
}

fn well_known_dart_sdk_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        out.push(PathBuf::from("C:/Program Files/Dart/dart-sdk"));
        out.push(PathBuf::from("C:/tools/dart-sdk"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(PathBuf::from(local).join("Pub").join("dart-sdk"));
        }
    }
    out.push(PathBuf::from("/usr/lib/dart"));
    out.push(PathBuf::from("/usr/local/lib/dart"));
    out.push(PathBuf::from("/opt/dart-sdk"));
    out.push(PathBuf::from("/opt/homebrew/opt/dart/libexec"));
    out
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

fn walk_dart_sdk(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    for lib_name in DART_SDK_LIBS {
        let sub = dep.root.join(lib_name);
        if sub.is_dir() {
            walk_sdk_dir(&sub, &dep.root, dep, &mut out, 0);
        }
    }
    out
}

fn walk_sdk_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= 6 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue; };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') { continue; }
                if matches!(name, "test" | "tests") { continue; }
            }
            walk_sdk_dir(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".dart") { continue; }
            let rel = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:dart-sdk:{}", rel),
                absolute_path: path,
                language: "dart",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol index — shared with flutter_sdk
// ---------------------------------------------------------------------------

/// Build a `(module, name) → file` index over the given dep roots using
/// a header-only tree-sitter parse. Called by both `DartSdkEcosystem` and
/// `FlutterSdkEcosystem` (via `super::dart_sdk::build_dart_symbol_index`).
pub(super) fn build_dart_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        collect_dart_files_recursive(&dep.root, dep, &mut work);
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
            scan_dart_top_level(&src)
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

fn collect_dart_files_recursive(
    dir: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<(String, WalkedFile)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue; };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') || matches!(name, "test" | "tests") {
                    continue;
                }
            }
            collect_dart_files_recursive(&path, dep, out);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !name.ends_with(".dart") { continue; }
            let rel = path.to_string_lossy().replace('\\', "/");
            out.push((
                dep.module_path.clone(),
                WalkedFile {
                    relative_path: format!("ext:{}:{}", dep.ecosystem, rel),
                    absolute_path: path,
                    language: "dart",
                },
            ));
        }
    }
}

/// Header-only tree-sitter scan — top-level class/mixin/enum/extension/fn names.
fn scan_dart_top_level(source: &str) -> Vec<String> {
    let language: tree_sitter::Language = tree_sitter_dart::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_dart_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_dart_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "class_declaration"
        | "class_definition"
        | "mixin_declaration"
        | "enum_declaration"
        | "extension_declaration"
        | "function_signature"
        | "function_declaration"
        | "getter_signature"
        | "setter_signature"
        | "type_alias" => {
            if let Some(name_node) = node
                .child_by_field_name("name")
                .or_else(|| find_first_identifier(node))
            {
                if let Ok(t) = name_node.utf8_text(bytes) {
                    out.push(t.to_string());
                }
            }
        }
        _ => {}
    }
}

fn find_first_identifier<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(child);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let e = DartSdkEcosystem;
        assert_eq!(e.id(), ID);
        assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
        assert_eq!(Ecosystem::languages(&e), &["dart"]);
    }

    #[test]
    fn activation_is_language_present() {
        let e = DartSdkEcosystem;
        assert!(matches!(
            e.activation(),
            EcosystemActivation::LanguagePresent("dart")
        ));
    }

    #[test]
    fn supports_reachability_and_demand_driven() {
        let e = DartSdkEcosystem;
        assert!(Ecosystem::supports_reachability(&e));
        assert!(Ecosystem::uses_demand_driven_parse(&e));
    }

    #[test]
    fn locate_roots_empty_on_missing_sdk() {
        // Must not panic when Dart SDK is absent.
        let e = DartSdkEcosystem;
        let _ = Ecosystem::locate_roots(&e, &LocateContext {
            project_root: std::path::Path::new("."),
            manifests: &Default::default(),
            active_ecosystems: &[],
        });
    }

    #[test]
    fn walk_root_empty_on_bogus_dep() {
        let dep = ExternalDepRoot {
            module_path: "dart-sdk".to_string(),
            version: String::new(),
            root: PathBuf::from("/nonexistent/dart/sdk/lib"),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };
        let e = DartSdkEcosystem;
        assert!(Ecosystem::walk_root(&e, &dep).is_empty());
    }

    #[test]
    fn dart_sdk_libs_are_nonempty() {
        assert!(!DART_SDK_LIBS.is_empty());
        assert!(DART_SDK_LIBS.contains(&"core"));
        assert!(DART_SDK_LIBS.contains(&"async"));
    }

    #[test]
    fn scan_top_level_finds_class_and_function() {
        let src = r#"
class MyClass {}
abstract class Base {}
mixin Mixable {}
enum Color { red, green }
extension FooExt on int {}
void myFunction() {}
int get myGetter => 0;
"#;
        let names = scan_dart_top_level(src);
        assert!(names.contains(&"MyClass".to_string()), "should find MyClass");
        assert!(names.contains(&"Base".to_string()), "should find Base");
        assert!(names.contains(&"Mixable".to_string()), "should find Mixable");
        assert!(names.contains(&"Color".to_string()), "should find Color");
    }

    #[test]
    fn build_symbol_index_empty_on_no_roots() {
        let index = build_dart_symbol_index(&[]);
        assert!(index.is_empty());
    }
}
