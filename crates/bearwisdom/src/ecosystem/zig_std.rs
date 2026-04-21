// =============================================================================
// ecosystem/zig_std.rs — Zig stdlib ecosystem
//
// Probes the Zig installation's `lib/std/` directory via $ZIG_LIB_DIR,
// $ZIG_HOME/lib/std/, `which zig` parent traversal, and well-known platform
// paths. Walks depth-2 (covers std/*.zig and std/fs/*.zig submodules).
//
// Activation: Any([TransitiveOn(zig-pkg), LanguagePresent("zig")]) — active
// whenever a Zig project is detected. Degrade silently when no stdlib found.
// =============================================================================

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::symbol_index::SymbolLocationIndex;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("zig-std");
const LEGACY_ECOSYSTEM_TAG: &str = "zig-std";
const LANGUAGES: &[&str] = &["zig"];

pub struct ZigStdEcosystem;

impl Ecosystem for ZigStdEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::TransitiveOn(super::zig_pkg::ID),
            EcosystemActivation::LanguagePresent("zig"),
        ])
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_zig_stdlib()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_std_tree(dep)
    }

    fn supports_reachability(&self) -> bool { true }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        super::zig_pkg::build_zig_symbol_index_pub(dep_roots)
    }
}

impl ExternalSourceLocator for ZigStdEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_zig_stdlib()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_std_tree(dep)
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<ZigStdEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(ZigStdEcosystem)).clone()
}

// ===========================================================================
// Discovery
// ===========================================================================

fn discover_zig_stdlib() -> Vec<ExternalDepRoot> {
    let Some(std_dir) = probe_std_dir() else {
        debug!("zig-std: no stdlib dir found; degrading silently");
        return Vec::new();
    };
    debug!("zig-std: using {}", std_dir.display());
    vec![ExternalDepRoot {
        module_path: "std".to_string(),
        version: String::new(),
        root: std_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }]
}

fn probe_std_dir() -> Option<PathBuf> {
    // 1. Explicit override via ZIG_LIB_DIR (or BEARWISDOM_ZIG_STDLIB).
    for var in ["ZIG_LIB_DIR", "BEARWISDOM_ZIG_STDLIB"] {
        if let Ok(val) = std::env::var(var) {
            let p = PathBuf::from(val);
            if p.is_dir() { return Some(p); }
        }
    }

    // 2. $ZIG_HOME/lib/std or $ZIG_HOME/lib/zig/std.
    for var in ["ZIG_HOME", "ZIG_ROOT"] {
        if let Ok(val) = std::env::var(var) {
            let base = PathBuf::from(val);
            for sub in ["lib/std", "lib/zig/std"] {
                let p = base.join(sub);
                if p.is_dir() { return Some(p); }
            }
        }
    }

    // 3. `which zig` / `where zig` — walk parent dirs for lib/std.
    if let Some(dir) = find_zig_from_path() {
        return Some(dir);
    }

    // 4. Well-known platform paths.
    let candidates = platform_candidates();
    for p in candidates {
        if p.is_dir() { return Some(p); }
    }

    None
}

fn find_zig_from_path() -> Option<PathBuf> {
    // Try both "zig" and "zig.exe" to cover *nix and Windows in a unified way.
    for bin in ["zig", "zig.exe"] {
        let Ok(output) = Command::new(bin).arg("env").arg("lib").output() else {
            continue;
        };
        if output.status.success() {
            let s = String::from_utf8(output.stdout).ok()?;
            let lib = PathBuf::from(s.trim());
            let std_dir = lib.join("std");
            if std_dir.is_dir() { return Some(std_dir); }
            // Some builds put it at lib root.
            if lib.join("std.zig").is_file() { return Some(lib); }
        }
    }

    // Fallback: resolve the binary via `which` and walk up to find lib/std.
    let which_out = Command::new("which").arg("zig").output()
        .or_else(|_| Command::new("where").arg("zig").output())
        .ok()?;
    if !which_out.status.success() { return None; }
    let raw = String::from_utf8(which_out.stdout).ok()?;
    let bin_path = PathBuf::from(raw.lines().next()?.trim());
    let mut dir = bin_path.parent()?;
    // Walk up two levels max: bin/../lib/std  or  prefix/lib/std.
    for _ in 0..3 {
        for sub in ["lib/std", "lib/zig/std"] {
            let p = dir.join(sub);
            if p.is_dir() { return Some(p); }
        }
        dir = dir.parent()?;
    }
    None
}

fn platform_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();

    // Windows
    out.push(PathBuf::from("C:/zig/lib/std"));
    out.push(PathBuf::from("C:/zig/lib/zig/std"));

    // Linux
    out.push(PathBuf::from("/usr/lib/zig/std"));
    out.push(PathBuf::from("/usr/local/lib/zig/std"));
    out.push(PathBuf::from("/usr/share/zig/lib/std"));

    // macOS Homebrew (glob-style: try a few known versions)
    let brew_cellar = PathBuf::from("/opt/homebrew/Cellar/zig");
    if let Ok(entries) = std::fs::read_dir(&brew_cellar) {
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.path())
            .collect();
        versions.sort();
        for ver in versions.into_iter().rev().take(3) {
            out.push(ver.join("lib/std"));
            out.push(ver.join("lib/zig/std"));
        }
    }

    out
}

// ===========================================================================
// Walker — depth-2 over std/*.zig and std/**/*.zig submodules
// ===========================================================================

fn walk_std_tree(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_dir(&dep.root, &dep.root, &mut out, 0);
    out
}

fn walk_dir(dir: &Path, root: &Path, out: &mut Vec<WalkedFile>, depth: u32) {
    // depth 0 = std/, depth 1 = std/fs/ — at most one level of subdirectory.
    // depth 2 would be std/fs/sub/ which is deeper than needed.
    if depth > 1 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "debug") { continue; }
                if name.starts_with('.') { continue; }
            }
            walk_dir(&path, root, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".zig") { continue; }
            // Skip test files (test_*.zig, *_test.zig).
            if name.starts_with("test_") || name.ends_with("_test.zig") { continue; }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!("ext:zig-std:std/{rel}"),
                absolute_path: path,
                language: "zig",
            });
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Ecosystem identity
    // -----------------------------------------------------------------------

    #[test]
    fn ecosystem_id_and_kind() {
        let eco = ZigStdEcosystem;
        assert_eq!(eco.id(), ID);
        assert_eq!(eco.id().as_str(), "zig-std");
        assert_eq!(eco.kind(), EcosystemKind::Stdlib);
        assert_eq!(Ecosystem::languages(&eco), &["zig"]);
    }

    #[test]
    fn uses_demand_driven_and_reachability() {
        let eco = ZigStdEcosystem;
        assert!(eco.uses_demand_driven_parse());
        assert!(eco.supports_reachability());
    }

    // -----------------------------------------------------------------------
    // Synthetic stdlib directory — verify symbol resolution
    // -----------------------------------------------------------------------

    fn make_synthetic_stdlib(tmp: &Path) {
        // std/mem.zig
        let mem = tmp.join("mem.zig");
        std::fs::write(&mem, "\
pub const Allocator = struct {};\n\
pub const Alignment = u29;\n\
pub fn alloc(a: *Allocator, n: usize) ![]u8 { _ = a; _ = n; return error.OutOfMemory; }\n\
pub fn copy(comptime T: type, dest: []T, src: []const T) void { _ = dest; _ = src; }\n\
").unwrap();

        // std/fs.zig
        let fs = tmp.join("fs.zig");
        std::fs::write(&fs, "\
pub const File = struct {};\n\
pub const Dir = struct {};\n\
pub fn cwd() Dir { return .{}; }\n\
pub fn openFileAbsolute(path: []const u8, flags: File.OpenFlags) !File { _ = path; _ = flags; return .{}; }\n\
").unwrap();

        // std/array_list.zig (ArrayList lives here in the real stdlib)
        let al = tmp.join("array_list.zig");
        std::fs::write(&al, "\
pub fn ArrayList(comptime T: type) type { _ = T; return struct{}; }\n\
pub fn ArrayListUnmanaged(comptime T: type) type { _ = T; return struct{}; }\n\
").unwrap();

        // std/io.zig
        let io = tmp.join("io.zig");
        std::fs::write(&io, "\
pub const Writer = struct {};\n\
pub const Reader = struct {};\n\
pub fn getStdOut() Writer { return .{}; }\n\
").unwrap();

        // std/fs/ subdirectory (depth-1 check)
        std::fs::create_dir_all(tmp.join("fs")).unwrap();
        let fs_file = tmp.join("fs").join("file.zig");
        std::fs::write(&fs_file, "\
pub const OpenFlags = struct {};\n\
pub const CreateFlags = struct {};\n\
").unwrap();
    }

    #[test]
    fn synthetic_stdlib_walk_and_index() {
        let tmp = std::env::temp_dir().join("bw-test-zig-std-synth");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        make_synthetic_stdlib(&tmp);

        let dep = ExternalDepRoot {
            module_path: "std".to_string(),
            version: String::new(),
            root: tmp.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };

        // Walk must collect .zig files including subdirectory.
        let walked = walk_std_tree(&dep);
        let rel_paths: Vec<&str> = walked.iter().map(|f| f.relative_path.as_str()).collect();
        assert!(
            rel_paths.iter().any(|p| p.contains("mem.zig")),
            "mem.zig missing; got: {rel_paths:?}"
        );
        assert!(
            rel_paths.iter().any(|p| p.contains("fs.zig")),
            "fs.zig missing"
        );
        assert!(
            rel_paths.iter().any(|p| p.contains("array_list.zig")),
            "array_list.zig missing"
        );
        assert!(
            rel_paths.iter().any(|p| p.contains("fs/file.zig") || p.contains("fs\\file.zig")),
            "fs/file.zig (depth-1 submodule) missing; got: {rel_paths:?}"
        );

        // Build symbol index and verify key std symbols resolve.
        let eco = ZigStdEcosystem;
        let index = eco.build_symbol_index(&[dep]);

        // std.mem.Allocator
        let allocator_file = index.locate("std", "Allocator");
        assert!(
            allocator_file.is_some(),
            "std.mem.Allocator not in index (expected from mem.zig); index len={}",
            index.len()
        );

        // std.fs.File
        let file_sym = index.locate("std", "File");
        assert!(
            file_sym.is_some(),
            "std.fs.File not in index (expected from fs.zig)"
        );

        // std.ArrayList (array_list.zig top-level generic fn)
        let al_sym = index.locate("std", "ArrayList");
        assert!(
            al_sym.is_some(),
            "std.ArrayList not in index (expected from array_list.zig)"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn no_stdlib_degrades_silently() {
        // With no ZIG_LIB_DIR set and no system zig, locate_roots returns empty.
        // We can't guarantee a zig install is absent but we can verify the
        // function doesn't panic.
        let eco = ZigStdEcosystem;
        let manifests = Default::default();
        let ctx = LocateContext {
            project_root: std::path::Path::new("."),
            manifests: &manifests,
            active_ecosystems: &[],
        };
        let roots = Ecosystem::locate_roots(&eco, &ctx);
        // Either empty (no zig) or one root (zig present). Both are valid.
        assert!(roots.len() <= 1);
    }

    #[test]
    fn walk_skips_test_files_and_deep_dirs() {
        let tmp = std::env::temp_dir().join("bw-test-zig-std-skip");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Normal file — included.
        std::fs::write(tmp.join("mem.zig"), "pub const x = 1;\n").unwrap();

        // Test file — excluded.
        std::fs::write(tmp.join("test_mem.zig"), "test \"x\" {}\n").unwrap();

        // Depth-3 directory (std/a/b/) — excluded.
        let deep = tmp.join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.zig"), "pub const y = 2;\n").unwrap();

        let dep = ExternalDepRoot {
            module_path: "std".to_string(),
            version: String::new(),
            root: tmp.clone(),
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        };

        let walked = walk_std_tree(&dep);
        let names: Vec<&str> = walked
            .iter()
            .filter_map(|f| f.absolute_path.file_name()?.to_str())
            .collect();

        assert!(names.contains(&"mem.zig"), "mem.zig should be included");
        assert!(!names.contains(&"test_mem.zig"), "test_mem.zig should be excluded");
        assert!(!names.contains(&"deep.zig"), "depth-3 file should be excluded");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
