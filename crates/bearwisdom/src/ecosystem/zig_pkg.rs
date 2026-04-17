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

use std::path::Path;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
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
    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("zig"),
        ])
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
