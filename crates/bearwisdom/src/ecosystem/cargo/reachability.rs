// ---------------------------------------------------------------------------
// Reachability: crate entry + bounded `mod X;` expansion
// ---------------------------------------------------------------------------

use std::path::{Path, PathBuf};

use crate::ecosystem::externals::ExternalDepRoot;
use crate::walker::WalkedFile;

const RS_MOD_MAX_DEPTH: u32 = 3;

/// Start from `src/lib.rs` (falling back to `src/main.rs` for binary-only
/// crates) and recursively follow `mod X;` declarations into the matching
/// source files. Each file yields zero or more child modules; bounded at
/// depth 3 so deeply nested crates still get their top surface without
/// walking every internal module.
pub(super) fn resolve_crate_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    // Crates with a non-standard layout (`tree-sitter`'s
    // `binding_rust/lib.rs`, vendored embedded bindings, etc.) declare
    // their entry via `[lib] path = "..."` in Cargo.toml. Honor that
    // first; fall back to the conventional `src/lib.rs` / `src/main.rs`.
    // Without this branch, ~6-10% of cargo deps in a typical workspace
    // walk to zero files (every C-with-Rust-bindings crate, every crate
    // that carves up its workspace into custom directories).
    let entry = lib_entry_from_manifest(&dep.root)
        .or_else(|| {
            let src = dep.root.join("src");
            let lib = src.join("lib.rs");
            if lib.is_file() { Some(lib) }
            else {
                let main = src.join("main.rs");
                if main.is_file() { Some(main) } else { None }
            }
        });
    let Some(entry) = entry else { return Vec::new() };
    if !entry.is_file() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    expand_rust_mods_into(dep, &dep.root, &entry, &mut out, &mut seen, 0);
    out
}

/// Read `[lib] path = "..."` from a crate's Cargo.toml, returning the
/// resolved absolute path if the file exists. Cheap text scan — no toml
/// parser dependency, no allocation beyond the read string. Tolerates
/// the field appearing under either `[lib]` or `[package.metadata]`-style
/// tables; we only look for the literal `[lib]` table since that's where
/// crates publishing on crates.io put it.
pub(super) fn lib_entry_from_manifest(crate_root: &Path) -> Option<PathBuf> {
    let manifest = crate_root.join("Cargo.toml");
    let content = std::fs::read_to_string(&manifest).ok()?;
    let mut in_lib = false;
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with('#') { continue }
        if line.starts_with('[') && line.ends_with(']') {
            // Match `[lib]` exactly — not `[lib.something]` and not
            // `[[bin]]`. Anything else exits the [lib] table.
            in_lib = line == "[lib]";
            continue;
        }
        if !in_lib { continue }
        let Some(stripped) = line.strip_prefix("path") else { continue };
        let stripped = stripped.trim_start();
        let Some(rest) = stripped.strip_prefix('=') else { continue };
        let val = rest.trim();
        // Strip surrounding quotes.
        let Some(val) = val
            .strip_prefix('"').and_then(|s| s.strip_suffix('"'))
            .or_else(|| val.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        else { continue };
        let abs = crate_root.join(val);
        if abs.is_file() {
            return Some(abs);
        }
    }
    None
}

fn expand_rust_mods_into(
    dep: &ExternalDepRoot,
    crate_root: &Path,
    file: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !seen.insert(file.to_path_buf()) { return }
    if !file.is_file() { return }

    let rel_sub = match file.strip_prefix(crate_root) {
        Ok(p) => p.to_string_lossy().replace('\\', "/"),
        Err(_) => file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mod.rs")
            .to_string(),
    };
    out.push(WalkedFile {
        relative_path: format!("ext:rust:{}/{}", dep.module_path, rel_sub),
        absolute_path: file.to_path_buf(),
        language: "rust",
    });

    if depth >= RS_MOD_MAX_DEPTH { return }

    let Ok(src) = std::fs::read_to_string(file) else { return };
    for child in extract_rust_mod_decls(&src) {
        let Some(next) = resolve_rust_mod_path(file, &child) else { continue };
        expand_rust_mods_into(dep, crate_root, &next, out, seen, depth + 1);
    }
}

/// Scan line-oriented for `mod X;` and `pub mod X;` (including
/// `pub(crate) mod X;` and similar visibility modifiers). Inline `mod X {
/// ... }` bodies are skipped — their contents are already in the same file.
pub(super) fn extract_rust_mod_decls(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in src.lines() {
        let line = raw.trim_start();
        // Strip line comments.
        let line = match line.find("//") {
            Some(ix) => &line[..ix],
            None => line,
        };
        let line = line.trim();
        if !line.ends_with(';') { continue }

        let mut rest = line;
        if let Some(r) = rest.strip_prefix("pub") {
            rest = r.trim_start();
            // Optional visibility qualifier: pub(crate), pub(super), pub(in path)
            if let Some(r) = rest.strip_prefix('(') {
                let Some(close) = r.find(')') else { continue };
                rest = r[close + 1..].trim_start();
            }
        }

        let Some(r) = rest.strip_prefix("mod") else { continue };
        // Ensure the next char is whitespace — avoid matching `model;` etc.
        let after = r;
        if !after.starts_with(|c: char| c.is_whitespace()) { continue }
        let ident = after.trim_start().trim_end_matches(';').trim();
        if ident.is_empty() { continue }
        if !ident.chars().all(|c| c.is_alphanumeric() || c == '_') { continue }
        out.push(ident.to_string());
    }
    out
}

/// Resolve `mod X;` declared in `from_file` to either `X.rs` or `X/mod.rs`
/// relative to the owning module directory (file's parent for lib.rs/
/// main.rs/mod.rs; sibling directory named after the stem otherwise).
pub(super) fn resolve_rust_mod_path(from_file: &Path, child: &str) -> Option<PathBuf> {
    let parent = from_file.parent()?;
    let stem = from_file.file_stem().and_then(|s| s.to_str())?;
    let mod_dir = if stem == "lib" || stem == "main" || stem == "mod" {
        parent.to_path_buf()
    } else {
        parent.join(stem)
    };

    let as_file = mod_dir.join(format!("{child}.rs"));
    if as_file.is_file() { return Some(as_file) }
    let as_mod = mod_dir.join(child).join("mod.rs");
    if as_mod.is_file() { return Some(as_mod) }
    None
}
