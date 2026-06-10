// ---------------------------------------------------------------------------
// Walk: single recursive walker. Starts at dep.root; skips non-source
// directories; emits .ex/.exs/.erl/.hrl/.gleam with per-file language.
// ---------------------------------------------------------------------------

use std::path::Path;

use crate::ecosystem::externals::{ExternalDepRoot, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub(crate) fn walk_hex_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    // Start at conventional source subdirs when they exist; this matches
    // the behavior of the original per-language walkers:
    //   Elixir: lib/
    //   Erlang: src/ + include/
    //   Gleam:  src/ (fallback to root)
    // Walking the package root directly would pick up mix.exs,
    // rebar.config, and other build scripts that the per-language walkers
    // intentionally excluded.
    let mut any_subdir = false;
    for subdir in &["lib", "src", "include"] {
        let d = dep.root.join(subdir);
        if d.is_dir() {
            walk_dir_bounded(&d, &dep.root, dep, &mut out, 0);
            any_subdir = true;
        }
    }
    // Gleam packages may ship flat (no src/). Fall back to walking the root
    // when no conventional source subdir exists.
    if !any_subdir {
        walk_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    }
    out
}

fn walk_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "test"
                        | "tests"
                        | "priv"
                        | "bin"
                        | "config"
                        | "doc"
                        | "docs"
                        | "assets"
                        | "examples"
                        | "_build"
                        | "cover"
                        | "ebin"
                        | "deps"
                        | "target"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let (language, virtual_tag) = match detect_hex_language(name) {
                Some(spec) => spec,
                None => continue,
            };
            // Skip test-suffixed files.
            if name.ends_with("_SUITE.erl") || name.ends_with("_tests.erl") {
                continue;
            }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:{virtual_tag}:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

pub(super) fn detect_hex_language(name: &str) -> Option<(&'static str, &'static str)> {
    if name.ends_with(".ex") || name.ends_with(".exs") {
        Some(("elixir", "elixir"))
    } else if name.ends_with(".erl") || name.ends_with(".hrl") {
        Some(("erlang", "erlang"))
    } else if name.ends_with(".gleam") {
        Some(("gleam", "gleam"))
    } else {
        None
    }
}
