// indexer/module_resolution/dart_mod.rs — Dart module resolver
//
// Resolution rules:
//   1. Relative (`./x.dart`, `../util.dart`, bare `x.dart`) → resolve against
//      the importing file's directory, suffix-match against indexed paths.
//   2. `package:<self>/x.dart` → the project's own package. Strip the self
//      package prefix and map to the package's `lib/` directory
//      (`package:app/x.dart` → `lib/x.dart`), then suffix-match.
//   3. `dart:` and `package:<other>/...` → external. Return `None` and leave
//      the ref for `classify_external` to brand.

use super::ModuleResolver;

/// Resolver for Dart import URIs.
///
/// `self_package` is the package name declared in the project's `pubspec.yaml`
/// (`name:`). It is the only signal that distinguishes a `package:<self>/...`
/// URI (project-local) from a `package:<other>/...` URI (third-party). When
/// `None`, every `package:` URI is treated as external.
pub struct DartModuleResolver {
    self_package: Option<String>,
}

const LANGUAGES: &[&str] = &["dart"];

impl DartModuleResolver {
    pub fn new(self_package: Option<String>) -> Self {
        Self { self_package }
    }
}

impl ModuleResolver for DartModuleResolver {
    fn language_ids(&self) -> &[&str] {
        LANGUAGES
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        if specifier.is_empty() || specifier.starts_with("dart:") {
            return None;
        }

        if let Some(pkg_path) = specifier.strip_prefix("package:") {
            // `package:<pkg>/<rest>` — only the project's own package resolves
            // locally; foreign packages are external.
            let self_pkg = self.self_package.as_deref()?;
            let prefix = format!("{}/", self_pkg);
            let rest = pkg_path.strip_prefix(&prefix)?;
            // A package URI's path is rooted at the package's `lib/` directory.
            let candidate = format!("lib/{}", rest);
            return find_dart_file(&candidate, file_paths);
        }

        // Relative or bare specifier — resolve against the importing file's
        // directory.
        let base = join_paths(parent_dir(importing_file), specifier);
        let base = normalise_path(&base);
        find_dart_file(&base, file_paths)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the directory portion of `file_path` (forward-slash normalised).
fn parent_dir(file_path: &str) -> &str {
    if let Some(pos) = file_path.rfind(|c| c == '/' || c == '\\') {
        &file_path[..pos]
    } else {
        "."
    }
}

/// Join a directory and a (possibly relative) path, normalising separators.
fn join_paths(dir: &str, tail: &str) -> String {
    let dir = dir.replace('\\', "/");
    let tail = tail.replace('\\', "/");
    if dir.is_empty() || dir == "." {
        tail
    } else {
        format!(
            "{}/{}",
            dir.trim_end_matches('/'),
            tail.trim_start_matches('/')
        )
    }
}

/// Collapse `..` components and drop redundant `.` segments.
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

/// Find `candidate` (already carrying its `.dart` extension) as an exact or
/// path-suffix match against the indexed file set.
fn find_dart_file(candidate: &str, file_paths: &[&str]) -> Option<String> {
    if candidate.is_empty() {
        return None;
    }
    for &p in file_paths {
        let norm = p.replace('\\', "/");
        if norm == candidate || norm.ends_with(&format!("/{}", candidate)) {
            return Some(p.to_string());
        }
    }
    None
}

#[cfg(test)]
#[path = "dart_mod_tests.rs"]
mod tests;
