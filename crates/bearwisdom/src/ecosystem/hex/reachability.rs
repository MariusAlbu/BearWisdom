// ---------------------------------------------------------------------------
// R3 reachability — scan project for module references, narrow walk
// ---------------------------------------------------------------------------
//
// Each language uses a different convention but they all map module names
// onto file paths:
//   - Elixir: `alias Foo.Bar` / `import Foo.Bar` / `use Foo.Bar` / `Foo.Bar.fn()`
//             → file `foo/bar.ex` under `lib/`
//   - Erlang: `foo:bar()` / `-include("foo.hrl").` → `foo.erl` / `foo.hrl`
//   - Gleam:  `import foo/bar` → `foo/bar.gleam` under `src/`
//
// We collect every module reference once across the project and store the
// raw set on each ExternalDepRoot. walk_hex_narrowed maps each reference to
// candidate path tails and keeps only files matching them, plus same-dir
// siblings (same-module-namespace types/functions don't get a fresh
// reference but still need walking).

use std::path::{Path, PathBuf};

use super::walk::{detect_hex_language, walk_hex_root};
use crate::ecosystem::externals::{ExternalDepRoot, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

pub(super) fn collect_hex_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    scan_hex_imports_recursive(project_root, &mut out, 0);
    out
}

fn scan_hex_imports_recursive(
    dir: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    ".git"
                        | "deps"
                        | "_build"
                        | "node_modules"
                        | "build"
                        | "priv"
                        | "ebin"
                        | "cover"
                        | "doc"
                        | "docs"
                        | "assets"
                        | "tmp"
                        | "target"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            scan_hex_imports_recursive(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if name.ends_with(".ex") || name.ends_with(".exs") {
                extract_elixir_module_refs(&content, out);
            } else if name.ends_with(".erl") || name.ends_with(".hrl") {
                extract_erlang_module_refs(&content, out);
            } else if name.ends_with(".gleam") {
                extract_gleam_module_refs(&content, out);
            }
        }
    }
}

/// Capture `alias Foo.Bar` / `alias Foo.{Bar, Baz}` / `import Foo` / `use Foo` /
/// `Foo.Bar.fun()` / `%Foo.Bar{}`. Stored as Elixir module names (dotted) — the
/// narrowing pass converts each to a `lib/foo/bar.ex` tail.
pub(super) fn extract_elixir_module_refs(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    for raw in content.lines() {
        let line = raw.trim();
        // `alias Foo.{Bar, Baz}`
        if let Some(rest) = line.strip_prefix("alias ") {
            collect_elixir_dotted_or_braced(rest, out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("import ") {
            collect_elixir_dotted_or_braced(rest, out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("use ") {
            collect_elixir_dotted_or_braced(rest, out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ") {
            collect_elixir_dotted_or_braced(rest, out);
            continue;
        }
        // Inline references (`Foo.Bar.func`, `%Foo.Bar{}`). Walk the line for
        // capitalised dotted runs. Conservative — false positives just walk
        // an extra file, which is the failure mode we tolerate.
        scan_elixir_module_tokens(line, out);
    }
}

fn collect_elixir_dotted_or_braced(rest: &str, out: &mut std::collections::HashSet<String>) {
    let rest = rest.trim();
    // Brace block first (before any `,` split, since the block itself contains commas).
    if let Some(brace_open) = rest.find('{') {
        if let Some(brace_close) = rest.find('}') {
            let prefix = rest[..brace_open].trim_end_matches('.').trim();
            if prefix.is_empty() {
                return;
            }
            let inner = &rest[brace_open + 1..brace_close];
            for sel in inner.split(',') {
                let sel = sel.trim();
                if sel.is_empty() {
                    continue;
                }
                out.insert(format!("{prefix}.{sel}"));
            }
            return;
        }
    }
    // Single dotted name: stop at the first `,`/whitespace/options keyword.
    let head = rest
        .split(|c: char| c == ',' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim_end_matches(',');
    if !head.is_empty()
        && head
            .chars()
            .next()
            .map_or(false, |c| c.is_ascii_uppercase())
    {
        out.insert(head.to_string());
    }
}

fn scan_elixir_module_tokens(line: &str, out: &mut std::collections::HashSet<String>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            let tok = &line[start..i];
            if tok.contains('.')
                && tok.split('.').all(|seg| {
                    !seg.is_empty() && seg.chars().next().map_or(false, |c| c.is_ascii_uppercase())
                })
            {
                out.insert(tok.to_string());
            }
        } else {
            i += 1;
        }
    }
}

/// Erlang module references appear as `foo:bar(...)` calls and
/// `-include("foo.hrl").` directives. Stored as bare module/header names.
pub(super) fn extract_erlang_module_refs(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    for raw in content.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("-include(\"") {
            if let Some(end) = rest.find('"') {
                let header = &rest[..end];
                if !header.is_empty() {
                    out.insert(header.to_string());
                }
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("-include_lib(\"") {
            if let Some(end) = rest.find('"') {
                let header = &rest[..end];
                if let Some(slash) = header.rfind('/') {
                    out.insert(header[slash + 1..].to_string());
                } else {
                    out.insert(header.to_string());
                }
            }
            continue;
        }
        // `foo:bar(...)` — only first-segment matters; header tokens get the
        // bare module name (`foo`).
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b.is_ascii_lowercase() {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b':' {
                    let module = &line[start..i];
                    if !module.is_empty() {
                        out.insert(format!("{module}.erl"));
                    }
                }
            } else {
                i += 1;
            }
        }
    }
}

/// Gleam imports: `import foo/bar` → store as `foo/bar` (path-shaped).
pub(super) fn extract_gleam_module_refs(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    for raw in content.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("import ") else {
            continue;
        };
        let head = rest.split_whitespace().next().unwrap_or("");
        let head = head.split('.').next().unwrap_or("");
        if head.is_empty() {
            continue;
        }
        out.insert(format!("gleam:{head}"));
    }
}

/// Build the set of file path tails the narrow walk should match. We expand
/// each requested ref into language-specific candidate tails so a single
/// walked file can satisfy multiple convention checks.
pub(super) fn requested_to_path_suffixes(refs: &[String]) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for r in refs {
        // Gleam-tagged refs: `gleam:foo/bar` → `foo/bar.gleam`
        if let Some(path) = r.strip_prefix("gleam:") {
            out.insert(format!("{}.gleam", path.replace('.', "/")));
            continue;
        }
        // Erlang refs already carry an extension.
        if r.ends_with(".erl") || r.ends_with(".hrl") {
            out.insert(r.clone());
            continue;
        }
        // Elixir module: `Foo.Bar.Baz` → `lib/foo/bar/baz.ex`. We emit two
        // tails: the snake_cased file path AND each parent path so deep
        // modules still match when only a leaf file holds the dep.
        let snake = r
            .split('.')
            .map(elixir_to_snake)
            .collect::<Vec<_>>()
            .join("/");
        if !snake.is_empty() {
            out.insert(format!("{snake}.ex"));
            out.insert(format!("{snake}.exs"));
        }
    }
    out
}

/// `FooBarBaz` → `foo_bar_baz`. Elixir module-to-filename convention.
fn elixir_to_snake(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len() + 4);
    for (i, ch) in seg.char_indices() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_')
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub(super) fn walk_hex_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() {
        return walk_hex_root(dep);
    }
    let suffixes = requested_to_path_suffixes(&dep.requested_imports);
    if suffixes.is_empty() {
        return walk_hex_root(dep);
    }

    let mut out = Vec::new();
    let mut any_subdir = false;
    for subdir in &["lib", "src", "include"] {
        let d = dep.root.join(subdir);
        if d.is_dir() {
            walk_narrowed_dir(&d, &dep.root, dep, &suffixes, &mut out, 0);
            any_subdir = true;
        }
    }
    if !any_subdir {
        walk_narrowed_dir(&dep.root, &dep.root, dep, &suffixes, &mut out, 0);
    }
    out
}

fn walk_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    suffixes: &std::collections::HashSet<String>,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dir_files: Vec<(PathBuf, String, &'static str, &'static str)> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut any_match = false;

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
            subdirs.push(path);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some((language, virtual_tag)) = detect_hex_language(name) else {
                continue;
            };
            if name.ends_with("_SUITE.erl") || name.ends_with("_tests.erl") {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if suffixes.iter().any(|s| rel_sub.ends_with(s)) {
                any_match = true;
            }
            dir_files.push((path, rel_sub, language, virtual_tag));
        }
    }

    if any_match {
        for (path, rel_sub, language, virtual_tag) in dir_files {
            let virtual_path = format!("ext:{virtual_tag}:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }

    for sub in subdirs {
        walk_narrowed_dir(&sub, root, dep, suffixes, out, depth + 1);
    }
}
