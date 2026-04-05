// indexer/module_resolution/rust_mod.rs — Rust module resolver
//
// Maps Rust module paths (from `use` / `mod` statements) to file paths.
//
// Resolution rules:
//   1. `crate::foo::bar` → strip `crate::`, convert `::` → `/`,
//      try `foo/bar.rs` and `foo/bar/mod.rs` (anywhere in file_paths).
//   2. `self::foo`       → relative to the current module file's directory.
//   3. `super::foo`      → relative to the parent directory.
//   4. Any other path    → strip leading `::` if present, then rule 1 logic.

use super::ModuleResolver;

pub struct RustModuleResolver;

const LANGUAGES: &[&str] = &["rust"];

impl ModuleResolver for RustModuleResolver {
    fn language_ids(&self) -> &[&str] {
        LANGUAGES
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        let import_dir = parent_dir(importing_file);

        if let Some(tail) = specifier.strip_prefix("crate::") {
            // Absolute from crate root.
            let rel = tail.replace("::", "/");
            return find_rust_module(&rel, file_paths);
        }

        if let Some(tail) = specifier.strip_prefix("self::") {
            let rel = tail.replace("::", "/");
            let base = if import_dir.is_empty() {
                rel.clone()
            } else {
                format!("{}/{}", import_dir, rel)
            };
            return find_rust_module_path(&normalise_path(&base), file_paths);
        }

        if let Some(tail) = specifier.strip_prefix("super::") {
            let rel = tail.replace("::", "/");
            let parent = parent_dir(import_dir);
            let base = if parent.is_empty() {
                rel.clone()
            } else {
                format!("{}/{}", parent, rel)
            };
            return find_rust_module_path(&normalise_path(&base), file_paths);
        }

        // Strip leading `::` (absolute path, less common).
        let trimmed = specifier.trim_start_matches("::");
        // Only resolve if it looks like an internal module path.
        if trimmed.contains("::") || !trimmed.is_empty() {
            let rel = trimmed.replace("::", "/");
            return find_rust_module(&rel, file_paths);
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parent_dir(file_path: &str) -> &str {
    if let Some(pos) = file_path.rfind(|c| c == '/' || c == '\\') {
        &file_path[..pos]
    } else {
        ""
    }
}

fn normalise_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let mut out: Vec<&str> = Vec::with_capacity(parts.len());
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

/// Find a Rust module by its relative path anywhere in `file_paths`.
/// Tries `<rel>.rs` and `<rel>/mod.rs` as suffix matches.
fn find_rust_module(rel: &str, file_paths: &[&str]) -> Option<String> {
    let candidate_rs = format!("{}.rs", rel);
    let candidate_mod = format!("{}/mod.rs", rel);

    for &p in file_paths {
        let norm = p.replace('\\', "/");
        if norm.ends_with(&candidate_rs) || norm == candidate_rs {
            return Some(p.to_string());
        }
        if norm.ends_with(&candidate_mod) || norm == candidate_mod {
            return Some(p.to_string());
        }
    }
    None
}

/// Find a Rust module by exact resolved path (for self:: / super:: cases).
fn find_rust_module_path(base: &str, file_paths: &[&str]) -> Option<String> {
    let candidate_rs = format!("{}.rs", base);
    let candidate_mod = format!("{}/mod.rs", base);

    for &p in file_paths {
        let norm = p.replace('\\', "/");
        if norm == candidate_rs || norm == candidate_mod {
            return Some(p.to_string());
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

    fn resolve(spec: &str, from: &str, files: &[&str]) -> Option<String> {
        RustModuleResolver.resolve_to_file(spec, from, files)
    }

    #[test]
    fn crate_absolute() {
        let files = &["src/services/user.rs"];
        assert_eq!(
            resolve("crate::services::user", "src/main.rs", files),
            Some("src/services/user.rs".into())
        );
    }

    #[test]
    fn crate_mod_rs_fallback() {
        let files = &["src/services/mod.rs"];
        assert_eq!(
            resolve("crate::services", "src/main.rs", files),
            Some("src/services/mod.rs".into())
        );
    }

    #[test]
    fn self_relative() {
        let files = &["src/api/handlers.rs"];
        assert_eq!(
            resolve("self::handlers", "src/api/mod.rs", files),
            Some("src/api/handlers.rs".into())
        );
    }

    #[test]
    fn super_relative() {
        let files = &["src/types.rs"];
        assert_eq!(
            resolve("super::types", "src/api/handlers.rs", files),
            Some("src/types.rs".into())
        );
    }
}
