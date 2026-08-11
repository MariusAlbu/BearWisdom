// =============================================================================
// ecosystem/maven/reachability.rs
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::{debug, warn};
use tree_sitter::{Node, Parser};

use super::{detect_jvm_language, walk_maven_root};
use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, MAX_WALK_DEPTH};
use crate::walker::WalkedFile;

// ---------------------------------------------------------------------------
// R3 reachability — user-import scanner + package-narrowed walk
// ---------------------------------------------------------------------------
//
// JVM languages use fully-qualified `import a.b.C` statements (or `import a.b.*`
// wildcards). We scan every .java/.kt/.scala/.clj*/.groovy file in the project,
// extract the imports, and carry them on every Maven ExternalDepRoot. At walk
// time we narrow to only the package dirs the project actually references —
// e.g., a project that imports `org.springframework.context.ApplicationContext`
// causes spring-context's sources jar to yield only files under
// `org/springframework/context/` (~20 files) instead of the whole jar (~500
// files). Unreferenced artifacts contribute zero walked files, which is the
// expected outcome when a transitive dep is pulled in but never named.

pub(crate) fn collect_jvm_user_imports(project_root: &Path) -> std::collections::HashSet<String> {
    let mut imports = std::collections::HashSet::new();
    scan_jvm_imports_recursive(project_root, &mut imports, 0);
    imports
}

fn scan_jvm_imports_recursive(
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
                        | "target"
                        | "build"
                        | "out"
                        | "dist"
                        | "node_modules"
                        | ".gradle"
                        | ".idea"
                        | ".settings"
                        | "bin"
                        | ".cpcache"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            scan_jvm_imports_recursive(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let is_jvm = name.ends_with(".java")
                || name.ends_with(".kt")
                || name.ends_with(".kts")
                || name.ends_with(".scala")
                || name.ends_with(".clj")
                || name.ends_with(".cljc")
                || name.ends_with(".cljs")
                || name.ends_with(".groovy")
                || name.ends_with(".gradle");
            if !is_jvm {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if name.ends_with(".clj") || name.ends_with(".cljc") || name.ends_with(".cljs") {
                extract_clojure_imports(&content, out);
            } else {
                extract_jvm_imports_from_source(&content, out);
            }
        }
    }
}

/// Parse `import a.b.C;` / `import static a.b.C.method;` / `import a.b.{X, Y}` /
/// `import a.b.C` (Kotlin/Scala — no trailing `;`). Inserts the fully-qualified
/// name(s) without the trailing `;` or `{...}` pieces. Scala selector blocks
/// (`import a.b.{X, Y}`) emit `a.b.X` and `a.b.Y` so narrowing sees each class.
pub(crate) fn extract_jvm_imports_from_source(
    content: &str,
    out: &mut std::collections::HashSet<String>,
) {
    for raw in content.lines() {
        let line = raw.trim();
        // Java/Kotlin/Groovy form.
        let after_import = line
            .strip_prefix("import ")
            .or_else(|| line.strip_prefix("import\t"));
        let Some(rest) = after_import else { continue };
        let rest = rest.trim_start_matches("static ").trim();
        let rest = rest.trim_end_matches(';').trim();
        if rest.is_empty() {
            continue;
        }

        // Scala selector form: `a.b.{X, Y}` or `a.b.{X => Z}`.
        if let Some(brace_open) = rest.find('{') {
            if let Some(brace_close) = rest.find('}') {
                let prefix = rest[..brace_open].trim_end_matches('.').trim();
                if prefix.is_empty() {
                    continue;
                }
                let inner = &rest[brace_open + 1..brace_close];
                for sel in inner.split(',') {
                    let sel = sel.trim();
                    let head = sel.split("=>").next().unwrap_or("").trim();
                    if head.is_empty() || head == "_" {
                        out.insert(format!("{prefix}.*"));
                    } else {
                        out.insert(format!("{prefix}.{head}"));
                    }
                }
                continue;
            }
        }

        // Drop an `as alias` tail (Kotlin) or ` => alias` (Scala single).
        let head = rest
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(',');
        if head.is_empty() {
            continue;
        }
        out.insert(head.to_string());
    }
}

/// Parse Clojure `(:import [pkg.ns Class1 Class2])` / `(:import pkg.ns.Class)` /
/// `(:require [pkg.ns :as alias])` forms. Emits FQNs and ns namespaces — both
/// are used later by package-prefix narrowing.
pub(crate) fn extract_clojure_imports(content: &str, out: &mut std::collections::HashSet<String>) {
    // Minimal tolerant scanner — enough for the narrow-walk use. We're not
    // trying to be a Clojure reader; we just want dotted names following the
    // `:import` or `:require` keywords.
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':'
            && (i + 7 <= bytes.len() && &bytes[i..i + 7] == b":import"
                || i + 8 <= bytes.len() && &bytes[i..i + 8] == b":require")
        {
            let from = i;
            // Find the matching top-level close paren for this form.
            let mut depth: i32 = 0;
            let mut end = from;
            for (j, &b) in bytes.iter().enumerate().skip(from) {
                match b {
                    b'(' | b'[' => depth += 1,
                    b')' | b']' => {
                        depth -= 1;
                        if depth < 0 {
                            end = j;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if end == from {
                break;
            }
            let region = std::str::from_utf8(&bytes[from..end]).unwrap_or("");
            // `[pkg.ns Class1 Class2]` tokens, or `pkg.ns.Class`.
            for tok in region.split(|c: char| c.is_whitespace() || "()[]".contains(c)) {
                if tok.is_empty() {
                    continue;
                }
                if !tok.contains('.') {
                    continue;
                }
                if tok.starts_with(':') {
                    continue;
                }
                if tok.chars().next().map_or(true, |c| !c.is_alphabetic()) {
                    continue;
                }
                out.insert(tok.to_string());
            }
            i = end + 1;
        } else {
            i += 1;
        }
    }
}

/// Convert a user import like `com.foo.Bar` / `com.foo.*` / `com.foo.Outer.Inner`
/// to the on-disk directory prefix `com/foo/` (or `com/foo/Outer/` for nested).
/// Returns `None` for single-segment imports (no package, can't narrow).
pub(crate) fn jvm_import_to_package_prefix(fqn: &str) -> Option<String> {
    let trimmed = fqn.trim().trim_end_matches(".*");
    let last_dot = trimmed.rfind('.')?;
    let pkg = &trimmed[..last_dot];
    if pkg.is_empty() {
        return None;
    }
    Some(format!("{}/", pkg.replace('.', "/")))
}

pub(crate) fn walk_maven_narrowed(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if dep.requested_imports.is_empty() {
        // No demand data — legacy eager walk. Keeps a pre-R3-style answer.
        return walk_maven_root(dep);
    }
    let prefixes: std::collections::HashSet<String> = dep
        .requested_imports
        .iter()
        .filter_map(|fqn| jvm_import_to_package_prefix(fqn))
        .collect();
    if prefixes.is_empty() {
        return walk_maven_root(dep);
    }

    let mut out = Vec::new();
    walk_narrowed_dir(&dep.root, &dep.root, dep, &prefixes, &mut out, 0);
    out
}

fn walk_narrowed_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    prefixes: &std::collections::HashSet<String>,
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
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();

        let rel_sub = match path.strip_prefix(root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "META-INF") || name.starts_with('.') {
                    continue;
                }
            }
            // Recurse if this dir could still be a prefix of, or is under, a requested prefix.
            let dir_key = format!("{rel_sub}/");
            let may_contain = prefixes
                .iter()
                .any(|p| p.starts_with(&dir_key) || dir_key.starts_with(p));
            if !may_contain {
                continue;
            }
            walk_narrowed_dir(&path, root, dep, prefixes, out, depth + 1);
        } else if file_type.is_file() {
            // Match on the file's parent-dir key against any requested prefix.
            let Some(slash) = rel_sub.rfind('/') else {
                continue;
            };
            let parent_key = format!("{}/", &rel_sub[..slash]);
            if !prefixes
                .iter()
                .any(|p| parent_key == *p || parent_key.starts_with(p))
            {
                continue;
            }

            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let (language, virtual_tag) = match detect_jvm_language(name) {
                Some(spec) => spec,
                None => continue,
            };
            if name.ends_with("Test.java")
                || name.ends_with("Tests.java")
                || name.ends_with("Test.scala")
                || name.ends_with("Tests.scala")
                || name.ends_with("Spec.scala")
                || name.ends_with("Suite.scala")
                || name == "package-info.java"
                || name == "module-info.java"
            {
                continue;
            }

            let virtual_path = format!("ext:{virtual_tag}:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}
