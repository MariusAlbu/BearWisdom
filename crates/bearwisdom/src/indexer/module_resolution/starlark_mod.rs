// indexer/module_resolution/starlark_mod.rs — Starlark / Bazel module resolver
//
// Maps a Bazel `load()` label to an indexed `.bzl` file path:
//   1. `//pkg/sub:defs.bzl` — workspace-absolute. Strip the leading `//`,
//      turn the first `:` into `/`, suffix-match against the index
//      (`pkg/sub/defs.bzl`).
//   2. `:local.bzl` — same-package relative. Strip the leading `:`,
//      suffix-match the bare `.bzl` name.
//   3. `@repo//pkg:defs.bzl` — foreign repository. Return `None` and leave
//      the ref for `classify_external` to brand.

use super::{FilePathIndex, ModuleResolver};

/// Resolver for Bazel `load()` labels.
///
/// Stateless — `//` is unambiguously workspace-local, so no manifest signal is
/// needed to tell project-local labels from foreign-repo (`@`) ones.
pub struct StarlarkModuleResolver;

const LANGUAGES: &[&str] = &["starlark"];

impl StarlarkModuleResolver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StarlarkModuleResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Turn a workspace-local Bazel label into a repo-relative file path.
///
/// `//pkg:defs.bzl` → `pkg/defs.bzl`; `//a/b:c.bzl` → `a/b/c.bzl`;
/// `:local.bzl` → `local.bzl`. Only the first `:` is split — a label has at
/// most one target separator.
fn bazel_label_to_path(label: &str) -> String {
    let label = label.trim_start_matches("//");
    label.replacen(':', "/", 1)
}

impl ModuleResolver for StarlarkModuleResolver {
    fn language_ids(&self) -> &[&str] {
        LANGUAGES
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        _importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        let index = FilePathIndex::build(file_paths);
        self.resolve_to_file_indexed(specifier, _importing_file, &index)
    }

    fn resolve_to_file_indexed(
        &self,
        specifier: &str,
        _importing_file: &str,
        index: &FilePathIndex,
    ) -> Option<String> {
        if specifier.is_empty() || specifier.starts_with('@') {
            // Foreign repository — external. Leave it for classify_external.
            return None;
        }
        let candidate = bazel_label_to_path(specifier);
        if candidate.is_empty() {
            return None;
        }
        index.find_suffix(&candidate).map(str::to_string)
    }
}

#[cfg(test)]
#[path = "starlark_mod_tests.rs"]
mod tests;
