// =============================================================================
// indexer/module_resolution/nim_mod.rs — Nim module specifier resolver
// =============================================================================

use std::path::{Path, PathBuf};

use super::{FilePathIndex, ModuleResolver};

pub struct NimModuleResolver;

impl ModuleResolver for NimModuleResolver {
    fn language_ids(&self) -> &[&str] {
        &["nim"]
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        let index = FilePathIndex::build(file_paths);
        self.resolve_to_file_indexed(specifier, importing_file, &index)
    }

    fn resolve_to_file_indexed(
        &self,
        specifier: &str,
        importing_file: &str,
        index: &FilePathIndex,
    ) -> Option<String> {
        let spec = normalize_nim_specifier(specifier)?;

        if spec.starts_with('.') {
            return resolve_relative(&spec, importing_file, index);
        }

        if let Some(path) = resolve_from_importing_dir(&spec, importing_file, index) {
            return Some(path);
        }

        for candidate in bare_candidates(&spec) {
            if let Some(path) = index.find_suffix(&candidate) {
                return Some(path.to_string());
            }
        }

        None
    }
}

fn resolve_from_importing_dir(
    spec: &str,
    importing_file: &str,
    index: &FilePathIndex,
) -> Option<String> {
    let base = Path::new(importing_file)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let joined = normalize_path(base.join(spec));
    for candidate in file_candidates(&joined) {
        if let Some(path) = index.find_exact(&candidate) {
            return Some(path.to_string());
        }
    }
    None
}

fn normalize_nim_specifier(specifier: &str) -> Option<String> {
    let trimmed = specifier.trim().trim_matches('"').trim_matches('\'').trim();
    if trimmed.is_empty() {
        return None;
    }
    let compact = trimmed
        .replace('\\', "/")
        .split('/')
        .map(str::trim)
        .map(|s| s.trim_matches('"').trim_matches('\''))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if compact.is_empty() {
        None
    } else {
        Some(compact)
    }
}

fn resolve_relative(spec: &str, importing_file: &str, index: &FilePathIndex) -> Option<String> {
    let base = Path::new(importing_file)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let joined = normalize_path(base.join(spec));
    for candidate in file_candidates(&joined) {
        if let Some(path) = index.find_exact(&candidate) {
            return Some(path.to_string());
        }
        if let Some(path) = index.find_suffix(&candidate) {
            return Some(path.to_string());
        }
    }
    None
}

fn bare_candidates(spec: &str) -> Vec<String> {
    let stripped = spec
        .strip_prefix("std/")
        .or_else(|| spec.strip_prefix("pkg/"))
        .unwrap_or(spec);
    let leaf = stripped.rsplit('/').next().unwrap_or(stripped);

    let mut out = Vec::new();
    push_file_candidates(&mut out, stripped);
    if leaf != stripped {
        push_file_candidates(&mut out, leaf);
    }
    out
}

fn file_candidates(base: &str) -> Vec<String> {
    let mut out = Vec::new();
    push_file_candidates(&mut out, base);
    out
}

fn push_file_candidates(out: &mut Vec<String>, base: &str) {
    let base = base.trim_matches('/');
    if base.is_empty() {
        return;
    }
    if base.ends_with(".nim") {
        out.push(base.to_string());
    } else {
        out.push(format!("{base}.nim"));
        out.push(format!("{base}/mod.nim"));
    }
}

fn normalize_path(path: PathBuf) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        let s = component.as_os_str().to_string_lossy();
        match s.as_ref() {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other.replace('\\', "/")),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_nim_module() {
        let files = ["beacon_chain/foo.nim", "beacon_chain/spec/bar.nim"];
        let index = FilePathIndex::build(&files);
        let resolved =
            NimModuleResolver.resolve_to_file_indexed("./spec/bar", "beacon_chain/foo.nim", &index);
        assert_eq!(resolved.as_deref(), Some("beacon_chain/spec/bar.nim"));
    }

    #[test]
    fn resolves_quoted_relative_nim_module() {
        let files = [
            "beacon_chain/validator_client/api.nim",
            "beacon_chain/validator_client/common.nim",
        ];
        let index = FilePathIndex::build(&files);
        let resolved = NimModuleResolver.resolve_to_file_indexed(
            "\".\"/common",
            "beacon_chain/validator_client/api.nim",
            &index,
        );
        assert_eq!(
            resolved.as_deref(),
            Some("beacon_chain/validator_client/common.nim")
        );
    }

    #[test]
    fn resolves_external_bare_module_by_suffix() {
        let files = [
            "beacon_chain/foo.nim",
            "ext:submodule:vendor/nim-results/results.nim",
        ];
        let index = FilePathIndex::build(&files);
        let resolved =
            NimModuleResolver.resolve_to_file_indexed("results", "beacon_chain/foo.nim", &index);
        assert_eq!(
            resolved.as_deref(),
            Some("ext:submodule:vendor/nim-results/results.nim")
        );
    }

    #[test]
    fn prefers_sibling_module_before_global_suffix() {
        let files = [
            "other/common.nim",
            "beacon_chain/validator_client/api.nim",
            "beacon_chain/validator_client/common.nim",
        ];
        let index = FilePathIndex::build(&files);
        let resolved = NimModuleResolver.resolve_to_file_indexed(
            "common",
            "beacon_chain/validator_client/api.nim",
            &index,
        );
        assert_eq!(
            resolved.as_deref(),
            Some("beacon_chain/validator_client/common.nim")
        );
    }

    #[test]
    fn resolves_package_path_by_suffix() {
        let files = ["ext:nim:stew/stew/byteutils.nim"];
        let index = FilePathIndex::build(&files);
        let resolved = NimModuleResolver.resolve_to_file_indexed(
            "stew/byteutils",
            "beacon_chain/foo.nim",
            &index,
        );
        assert_eq!(resolved.as_deref(), Some("ext:nim:stew/stew/byteutils.nim"));
    }

    #[test]
    fn strips_std_prefix_for_stdlib_modules() {
        let files = ["ext:nim:nim-stdlib/pure/options.nim"];
        let index = FilePathIndex::build(&files);
        let resolved = NimModuleResolver.resolve_to_file_indexed(
            "std/options",
            "beacon_chain/foo.nim",
            &index,
        );
        assert_eq!(
            resolved.as_deref(),
            Some("ext:nim:nim-stdlib/pure/options.nim")
        );
    }
}
