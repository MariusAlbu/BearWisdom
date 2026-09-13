// ecosystem/manifest/gradle_build_logic.rs — Gradle DSL files outside
// `build.gradle[.kts]`.
//
// A Gradle project can declare dependencies in a build that is not a
// `build.gradle[.kts]` file at all: precompiled script plugins
// (`src/main/kotlin/*.gradle.kts`) and convention-plugin classes
// (`src/main/kotlin/*.kt`) inside `buildSrc` or an `includeBuild` target.
// Those declarations are the only place some artifacts appear, so the coord
// collector needs their file set alongside the ordinary build files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How many `settings.gradle[.kts]` hops the included-build search follows.
const MAX_SETTINGS_HOPS: usize = 2;

/// Directory-recursion bound inside one build's `src/main`.
const MAX_SOURCE_DEPTH: usize = 6;

/// Directory names that never hold authored DSL — dependency caches, VCS
/// metadata and build output (which contains Gradle's own generated copies
/// of the precompiled script plugins).
const PRUNED_DIRS: &[&str] = &[".git", "build", ".gradle", "target", "node_modules"];

/// Plugin ids that mark a build as producing Gradle plugins.
const PLUGIN_DEVELOPMENT_IDS: &[&str] =
    &["kotlin-dsl", "java-gradle-plugin", "groovy-gradle-plugin"];

/// Gradle DSL files that declare dependencies outside a `build.gradle[.kts]`:
/// precompiled script plugins and convention-plugin classes inside the
/// project's build-logic builds.
pub fn collect_gradle_build_logic_files(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for build_dir in build_logic_roots(project_root) {
        let include_jvm_sources = is_plugin_development_build(&build_dir);
        let source_root = build_dir.join("src").join("main");
        collect_dsl_sources(&source_root, include_jvm_sources, &mut out, 0);
    }
    out
}

/// Directories holding a project's build logic: the implicit `buildSrc` build
/// plus every `includeBuild("<path>")` target declared in a settings file.
/// Depth-bounded with a visited set; paths are resolved relative to the
/// settings file's own directory and must stay inside a normalized
/// `project_root`.
fn build_logic_roots(project_root: &Path) -> Vec<PathBuf> {
    let Ok(root) = project_root.canonicalize() else {
        return Vec::new();
    };

    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut frontier: Vec<PathBuf> = vec![root.clone()];

    for _ in 0..=MAX_SETTINGS_HOPS {
        let mut next: Vec<PathBuf> = Vec::new();
        for dir in frontier {
            if !visited.insert(dir.clone()) {
                continue;
            }
            let build_src = dir.join("buildSrc");
            if build_src.is_dir() && !out.contains(&build_src) {
                out.push(build_src);
            }
            for rel in included_build_paths(&dir) {
                let Ok(target) = dir.join(rel).canonicalize() else {
                    continue;
                };
                if !target.starts_with(&root) || !target.is_dir() {
                    continue;
                }
                if !out.contains(&target) {
                    out.push(target.clone());
                }
                next.push(target);
            }
        }
        frontier = next;
    }

    out
}

/// Included-build paths declared by the settings file of one build directory.
fn included_build_paths(dir: &Path) -> Vec<String> {
    for name in &["settings.gradle.kts", "settings.gradle"] {
        if let Ok(content) = std::fs::read_to_string(dir.join(name)) {
            return parse_included_build_paths(&content);
        }
    }
    Vec::new()
}

/// Included-build directory paths declared by one `settings.gradle[.kts]`
/// body. Accepts both the Kotlin `includeBuild("x")` and the Groovy
/// `includeBuild '../x'` call forms; subproject `include(":app")` directives
/// are not included builds and yield nothing here.
fn parse_included_build_paths(content: &str) -> Vec<String> {
    const KEYWORD: &str = "includeBuild";
    let mut out: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        let mut rest = trimmed;
        while let Some(idx) = rest.find(KEYWORD) {
            let before = rest[..idx].chars().next_back();
            let after = &rest[idx + KEYWORD.len()..];
            if !is_identifier_char(before) && starts_a_call_argument(after) {
                if let Some(path) = super::gradle::extract_quoted_literals(after)
                    .into_iter()
                    .next()
                {
                    if !path.is_empty() && !path.starts_with(':') && !out.contains(&path) {
                        out.push(path);
                    }
                }
            }
            rest = after;
        }
    }

    out
}

/// True when a character would make `includeBuild` the tail of a longer
/// identifier rather than a call of its own.
fn is_identifier_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// True when the text after a keyword opens its argument list, in either the
/// parenthesized or the bare Groovy call form.
fn starts_a_call_argument(after: &str) -> bool {
    matches!(
        after.chars().next(),
        Some('(') | Some(' ') | Some('\t') | Some('\'') | Some('"')
    )
}

/// True when the build publishes Gradle plugins — its JVM sources are build
/// logic rather than application code.
fn is_plugin_development_build(dir: &Path) -> bool {
    for name in &["build.gradle.kts", "build.gradle"] {
        if let Ok(content) = std::fs::read_to_string(dir.join(name)) {
            return declares_plugin_development(&content);
        }
    }
    false
}

/// True when a build script's `plugins { }` block applies one of the
/// plugin-development plugins.
fn declares_plugin_development(content: &str) -> bool {
    let Some(block) = plugins_block(content) else {
        return false;
    };
    PLUGIN_DEVELOPMENT_IDS.iter().any(|id| block.contains(*id))
}

/// The body of the first top-level `plugins { }` block, brace-matched.
fn plugins_block(content: &str) -> Option<String> {
    let start = content.find("plugins")?;
    let open = content[start..].find('{')? + start;
    let mut depth = 0usize;
    for (offset, c) in content[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(content[open + 1..open + offset].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// Collect `*.gradle`/`*.gradle.kts` unconditionally — the extension is the
/// marker — and `*.kt`/`*.groovy` only when `include_jvm_sources`.
fn collect_dsl_sources(
    dir: &Path,
    include_jvm_sources: bool,
    out: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > MAX_SOURCE_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if path.is_dir() {
            if PRUNED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_dsl_sources(&path, include_jvm_sources, out, depth + 1);
        } else if is_dsl_script(&name) || (include_jvm_sources && is_jvm_source(&name)) {
            out.push(path);
        }
    }
}

/// True for a Gradle build-script file name in either DSL.
fn is_dsl_script(name: &str) -> bool {
    name.ends_with(".gradle") || name.ends_with(".gradle.kts")
}

/// True for a JVM source file name that can hold a convention plugin.
fn is_jvm_source(name: &str) -> bool {
    name.ends_with(".kt") || name.ends_with(".groovy")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "gradle_build_logic_tests.rs"]
mod tests;
