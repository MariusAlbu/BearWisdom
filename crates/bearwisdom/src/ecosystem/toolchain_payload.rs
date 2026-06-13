// =============================================================================
// ecosystem/toolchain_payload.rs — manifest-less toolchain payload discovery
//
// Some language toolchains are distributed as a source checkout with no
// dependency manifest at the root: the shipped standard-library / vendored
// system-library subtrees are first-party payload, not application code. When
// such a checkout is indexed as a project, those subtrees must be classified
// `origin='external'` so the project's own resolution rate isn't measured
// against toolchain internals — mirroring how `vendored_submodules` marks
// `.gitmodules` subtrees.
//
// Unlike submodules there is no manifest to read, so detection is by the known
// folder layout of each toolchain. Each language has its own locator rule
// (gated on a layout marker that distinguishes the toolchain checkout from an
// ordinary application), and the public entry composes them — there is no
// shared cross-language path list.
//
// This module only supplies the external-classification signal consumed during
// origin assignment in `indexer/full.rs`; the files themselves are walked by
// the main scan.
// =============================================================================

use std::path::Path;

#[cfg(test)]
#[path = "toolchain_payload_tests.rs"]
mod tests;

/// Toolchain-payload subtree prefixes for the project at `project_root`,
/// normalized to forward slashes. Empty when the root is not a recognized
/// manifest-less toolchain checkout.
pub fn toolchain_payload_prefixes(project_root: &Path) -> Vec<String> {
    let mut out = zig_payload_prefixes(project_root);
    out.extend(odin_payload_prefixes(project_root));
    out
}

/// Zig toolchain checkout: `lib/std/` is the stdlib marker that identifies the
/// directory as a Zig install / compiler checkout (an ordinary Zig app has no
/// `lib/std`). Its sibling subtrees are vendored C/C++ payload shipped with the
/// toolchain, not Zig's own code: `lib/libc*` / `lib/libcxx*` (musl/libc++),
/// `lib/include` (bundled clang headers), and `lib/libtsan` (the compiler-rt
/// ThreadSanitizer runtime). `lib/std` itself stays internal — it is indexed via
/// the `zig-std` ecosystem.
fn zig_payload_prefixes(project_root: &Path) -> Vec<String> {
    if !project_root.join("lib").join("std").is_dir() {
        return Vec::new();
    }
    let lib = project_root.join("lib");
    let Ok(entries) = std::fs::read_dir(&lib) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_zig_vendored_payload_dir(&name) {
            out.push(format!("lib/{name}"));
        }
    }
    out
}

/// True for a `lib/<name>` subtree that holds vendored C/C++ payload bundled
/// with the Zig toolchain rather than Zig's own source: `libc`, `libcxx`,
/// `libcxxabi`, `libunwind` (the C/C++ system libraries), `include` (bundled
/// clang headers), and `libtsan` (the compiler-rt ThreadSanitizer runtime).
fn is_zig_vendored_payload_dir(name: &str) -> bool {
    name.starts_with("libc") || name.starts_with("libcxx") || matches!(name, "include" | "libtsan")
}

/// Odin toolchain checkout: `core/` (stdlib) and `vendor/` (bundled bindings)
/// at the root identify the directory as an Odin compiler checkout / install.
/// Both must be present — an ordinary Odin app may have a `core/` package of
/// its own but not the toolchain's `vendor/` tree alongside it.
fn odin_payload_prefixes(project_root: &Path) -> Vec<String> {
    let has_core = project_root.join("core").is_dir();
    let has_vendor = project_root.join("vendor").is_dir();
    if has_core && has_vendor {
        vec!["core".to_string(), "vendor".to_string()]
    } else {
        Vec::new()
    }
}

/// True when `rel_path` (project-root-relative) falls inside one of the
/// toolchain-payload subtrees. Matches the subtree exactly or any descendant —
/// a prefix sibling (`core` vs `corelib/x`) does NOT match.
pub fn is_under_toolchain_payload(rel_path: &str, prefixes: &[String]) -> bool {
    if prefixes.is_empty() {
        return false;
    }
    let norm = rel_path.replace('\\', "/");
    prefixes.iter().any(|p| {
        norm == *p
            || norm
                .strip_prefix(p.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}
