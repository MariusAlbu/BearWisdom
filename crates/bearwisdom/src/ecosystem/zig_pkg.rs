// =============================================================================
// ecosystem/zig_pkg.rs — Zig build.zig.zon ecosystem
//
// Phase 2 + 3: consolidates `indexer/externals/zig.rs` +
// `indexer/manifest/zig_zon.rs`. Zig fetches deps to `.zig-cache/p/<hash>/`
// — directory names are content hashes, not package names, so we match by
// reading `build.zig.zon` inside each hash dir to determine its name.
//
// Module named `zig_pkg` (not `zig`) because it's consistent with other
// keyword-avoiding names and clearer about intent.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::debug;
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::ecosystem::manifest::{ManifestData, ManifestKind, ManifestReader};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("zig-pkg");
const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["zig"];
const LEGACY_ECOSYSTEM_TAG: &str = "zig";

pub struct ZigPkgEcosystem;

impl Ecosystem for ZigPkgEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        &[
            ("build.zig",     "zig"),
            ("build.zig.zon", "zig"),
        ]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        &["zig-cache", "zig-out"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `build.zig.zon`. The Zig toolchain (stdlib)
        // belongs to `zig-std`; this ecosystem only resolves declared
        // third-party deps. Dropping the LanguagePresent shotgun is
        // correct per the trait doc.
        EcosystemActivation::ManifestMatch
    }
    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_zig_externals(ctx.project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_zig_root(dep) }
    fn supports_reachability(&self) -> bool { true }
    fn resolve_import(
        &self, dep: &ExternalDepRoot, _p: &str, _s: &[&str],
    ) -> Vec<WalkedFile> { walk_zig_narrowed(dep) }
    fn resolve_symbol(
        &self, dep: &ExternalDepRoot, _f: &str,
    ) -> Vec<WalkedFile> { walk_zig_narrowed(dep) }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_zig_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

impl ExternalSourceLocator for ZigPkgEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_zig_externals(project_root)
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> { walk_zig_root(dep) }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ZigPkgEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ZigPkgEcosystem)).clone()
}

// ===========================================================================
// Manifest reader (build.zig.zon)
// ===========================================================================

pub struct ZigZonManifest;

impl ManifestReader for ZigZonManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::ZigZon }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let zon = project_root.join("build.zig.zon");
        if !zon.is_file() { return None }
        let content = std::fs::read_to_string(&zon).ok()?;
        let mut data = ManifestData::default();
        for name in parse_zig_zon_deps(&content) {
            data.dependencies.insert(name);
        }
        Some(data)
    }
}

pub fn parse_zig_zon_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    let mut brace_depth = 0u32;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains(".dependencies") && trimmed.contains("= .{") {
            in_deps = true;
            brace_depth = 1;
            continue;
        }
        if !in_deps { continue }

        for ch in trimmed.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => {
                    brace_depth = brace_depth.saturating_sub(1);
                    if brace_depth == 0 { in_deps = false; }
                }
                _ => {}
            }
        }
        if brace_depth == 1 {
            if let Some(name) = extract_zon_dep_name(trimmed) {
                if !name.is_empty() { deps.push(name) }
            }
        }
    }
    deps
}

fn extract_zon_dep_name(line: &str) -> Option<String> {
    let trimmed = line.trim().trim_start_matches('.');
    if trimmed.starts_with("@\"") {
        let rest = &trimmed[2..];
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    if let Some(eq) = trimmed.find('=') {
        let name = trimmed[..eq].trim();
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Some(name.to_string());
        }
    }
    None
}

// ===========================================================================
// Discovery + walk
// ===========================================================================

pub fn discover_zig_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let zon = project_root.join("build.zig.zon");
    if !zon.is_file() { return Vec::new() }
    let Ok(content) = std::fs::read_to_string(&zon) else { return Vec::new() };
    let declared = parse_zig_zon_deps(&content);
    if declared.is_empty() { return Vec::new() }

    let cache = project_root.join(".zig-cache").join("p");
    if !cache.is_dir() { return Vec::new() }

    let user_imports: Vec<String> = collect_zig_user_imports(project_root)
        .into_iter()
        .collect();

    let Ok(entries) = std::fs::read_dir(&cache) else { return Vec::new() };
    let mut roots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue }
        let zon_path = path.join("build.zig.zon");
        if let Ok(zon_content) = std::fs::read_to_string(&zon_path) {
            if let Some(name) = extract_zig_zon_name(&zon_content) {
                if declared.iter().any(|d| d == &name) {
                    roots.push(ExternalDepRoot {
                        module_path: name,
                        version: String::new(),
                        root: path,
                        ecosystem: LEGACY_ECOSYSTEM_TAG,
                        package_id: None,
                        requested_imports: user_imports.clone(),
                    });
                }
            }
        }
    }
    debug!("Zig: {} external package roots", roots.len());
    roots
}

// R3 — `@import("X")` scanner + module-granular narrowed walk. Dep's
// module_path (set at discovery from the dep's own zon `.name = ...`)
// is matched against `@import` arguments; if any user `@import` names
// this dep, walk it fully — dep packages are typically small enough
// that intra-package narrowing isn't worth it, and Zig's `@import`
// targets a file-scoped namespace anyway.

fn collect_zig_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_zig_imports(project_root, &mut out, 0);
    out
}

fn scan_zig_imports(dir: &std::path::Path, out: &mut std::collections::HashSet<String>, depth: usize) {
    if depth > 12 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | ".zig-cache" | "zig-cache" | "zig-out" | "build")
                    || name.starts_with('.') { continue }
            }
            scan_zig_imports(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".zig") { continue }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            extract_zig_imports(&content, out);
        }
    }
}

fn extract_zig_imports(content: &str, out: &mut std::collections::HashSet<String>) {
    let bytes = content.as_bytes();
    let needle = b"@import(";
    let mut i = 0;
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            // Skip whitespace.
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') { j += 1; }
            if j < bytes.len() && bytes[j] == b'"' {
                let start = j + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'"' { end += 1; }
                if end < bytes.len() && start < end {
                    let arg = std::str::from_utf8(&bytes[start..end]).unwrap_or("").trim();
                    // We only care about declared dep names (alphanumeric+`_-`),
                    // not relative paths like `"foo/bar.zig"`.
                    if !arg.is_empty() && !arg.contains('/') && !arg.contains('\\') {
                        let trimmed = arg.trim_end_matches(".zig");
                        if !trimmed.is_empty() { out.insert(trimmed.to_string()); }
                    }
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
}

fn walk_zig_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() { return walk_zig_root(dep); }
    // Module-granular: walk the dep iff its name was @imported anywhere.
    if !dep.requested_imports.iter().any(|m| m == &dep.module_path) {
        return Vec::new();
    }
    walk_zig_root(dep)
}

fn extract_zig_zon_name(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(".name") {
            let rest = trimmed.splitn(2, '=').nth(1)?.trim();
            let name = rest.trim_start_matches('.').trim_matches(|c: char| c == ',' || c == '"' || c.is_whitespace());
            if !name.is_empty() { return Some(name.to_string()) }
        }
    }
    None
}

fn walk_zig_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
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
                if matches!(name, "test" | "tests" | "zig-cache") || name.starts_with('.') { continue }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".zig") { continue }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:zig:{}/{}", dep.module_path, rel_sub),
                absolute_path: path,
                language: "zig",
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------

pub(crate) fn build_zig_symbol_index_pub(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    build_zig_symbol_index(dep_roots)
}

fn build_zig_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_zig_root(dep) {
            work.push((dep.module_path.clone(), wf));
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
            scan_zig_header(&src)
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

/// Header-only tree-sitter scan of a Zig source file. Records top-level
/// `const Name = ...`, `var Name = ...`, `fn Name(...)` / `pub fn Name(...)`
/// declarations. Zig uses `const` for both type aliases and struct
/// definitions so types and functions end up in one bucket.
fn scan_zig_header(source: &str) -> Vec<String> {
    let language = tree_sitter_zig::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_zig_top_level_name(&child, bytes, &mut out);
    }
    out
}

fn collect_zig_top_level_name(node: &Node, bytes: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "FnProto" | "fn_decl" | "function_declaration" | "Decl" | "TopLevelDecl"
        | "VarDecl" | "variable_declaration" => {
            let mut name_node = node
                .child_by_field_name("name")
                .or_else(|| node.child_by_field_name("variable"));
            if name_node.is_none() {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "identifier" {
                        name_node = Some(child);
                        break;
                    }
                }
            }
            if let Some(name_node) = name_node {
                if let Ok(t) = name_node.utf8_text(bytes) {
                    out.push(t.to_string());
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        assert_eq!(ZigPkgEcosystem.id(), ID);
        assert_eq!(Ecosystem::languages(&ZigPkgEcosystem), &["zig"]);
    }

    #[test]
    fn parse_zig_zon() {
        let content = r#"
.{
    .name = .clap,
    .dependencies = .{
        .@"zig-clap" = .{ .url = "...", .hash = "..." },
        .known_folders = .{ .url = "...", .hash = "..." },
    },
}
"#;
        let deps = parse_zig_zon_deps(content);
        assert_eq!(deps, vec!["zig-clap", "known_folders"]);
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }

    #[test]
    fn zig_extracts_at_import_names() {
        let mut out = std::collections::HashSet::new();
        extract_zig_imports(
            "const std = @import(\"std\");\nconst clap = @import(\"clap\");\nconst local = @import(\"foo/bar.zig\");\n",
            &mut out,
        );
        assert!(out.contains("std"));
        assert!(out.contains("clap"));
        // Relative-path imports are project-internal, not dep names.
        assert!(!out.contains("foo/bar"));
    }

    #[test]
    fn zig_narrowed_walk_skips_unimported_deps() {
        let tmp = std::env::temp_dir().join("bw-test-zig-r3");
        let _ = std::fs::remove_dir_all(&tmp);
        let dep_root = tmp.join("clap-pkg");
        std::fs::create_dir_all(&dep_root).unwrap();
        std::fs::write(dep_root.join("clap.zig"), "// pkg\n").unwrap();

        // Imported: walk happens.
        let yes = ExternalDepRoot {
            module_path: "clap".to_string(),
            version: String::new(),
            root: dep_root.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: vec!["clap".to_string()],
        };
        assert_eq!(walk_zig_narrowed(&yes).len(), 1);

        // Not imported: skip.
        let no = ExternalDepRoot {
            module_path: "clap".to_string(),
            version: String::new(),
            root: dep_root.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: vec!["other".to_string()],
        };
        assert!(walk_zig_narrowed(&no).is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
