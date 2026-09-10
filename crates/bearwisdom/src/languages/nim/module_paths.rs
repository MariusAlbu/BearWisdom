// =============================================================================
// languages/nim/module_paths — Nim source-module path evidence
// =============================================================================

use crate::type_checker::profile::language_profile::{
    ModuleMatchAuthority, ModulePathMatch, ModuleSpecifierClass, SourceModulePathPolicy,
};

/// Adapt Nim module text into the generic resolver's file-path evidence.
pub(crate) fn module_path_match(module: &str) -> ModulePathMatch {
    ModulePathMatch {
        module_path: module.replace('\\', "/"),
        path_variants: Vec::new(),
        required_file_prefix: None,
        compound_extensions: &[],
        authority: ModuleMatchAuthority::Heuristic,
        source_module_path_policy: NIM_SOURCE_MODULE_PATH_POLICY,
    }
}

/// The ordinary module-prefix anchor has no Nim-specific rewrite candidates.
pub(crate) fn module_prefix_candidates(module: &str) -> Vec<String> {
    vec![module.to_string()]
}

/// Nim's adapter does not need to reject generic directory containment.
pub(crate) fn declines_directory_match(_module: &str) -> bool {
    false
}

/// Nim module path rules for the generic re-export walker.
pub(crate) const NIM_SOURCE_MODULE_PATH_POLICY: SourceModulePathPolicy = SourceModulePathPolicy {
    classify_specifier: classify_module_specifier,
    relative_candidate_paths: relative_reexport_candidate_paths,
    bare_module_matches_file,
    external_import_match_terms,
};

fn classify_module_specifier(specifier: &str) -> ModuleSpecifierClass {
    if specifier.starts_with('.')
        || specifier.starts_with('/')
        || (specifier.len() >= 2 && specifier.as_bytes()[1] == b':')
    {
        ModuleSpecifierClass::Relative
    } else if specifier.is_empty() {
        ModuleSpecifierClass::Unsupported
    } else {
        ModuleSpecifierClass::Bare
    }
}

/// A Nim relative module can name a concrete source file or its `mod` entry.
pub(crate) fn relative_reexport_candidate_paths(base: &str) -> Vec<String> {
    vec![
        base.to_string(),
        format!("{base}.nim"),
        format!("{base}/mod.nim"),
    ]
}

/// Match Nim's standard-library/package module spellings to indexed file paths.
pub(crate) fn bare_module_matches_file(file_path: &str, source_module: &str) -> bool {
    let trimmed = source_module.trim_matches('"').trim_matches('\'').trim();
    let stripped = trimmed
        .strip_prefix("std/")
        .or_else(|| trimmed.strip_prefix("pkg/"))
        .unwrap_or(trimmed)
        .replace('\\', "/");
    let module = stripped.trim_matches('/');
    if module.is_empty() {
        return false;
    }
    let normalized = file_path.replace('\\', "/");
    let candidates = if module.ends_with(".nim") {
        vec![module.to_string()]
    } else {
        vec![format!("{module}.nim"), format!("{module}/mod.nim")]
    };
    candidates
        .iter()
        .any(|candidate| normalized == *candidate || normalized.ends_with(&format!("/{candidate}")))
}

/// External-import terms after applying Nim's standard-library/package roots.
pub(crate) fn external_import_match_terms(module: &str) -> Vec<String> {
    let stripped = module
        .strip_prefix("std/")
        .or_else(|| module.strip_prefix("pkg/"))
        .unwrap_or(module);
    let mut terms = Vec::new();
    let leaf = stripped.rsplit('/').next().unwrap_or(stripped);
    if !leaf.is_empty() {
        terms.push(leaf.to_lowercase());
    }
    let root = module.split('/').next().unwrap_or(module);
    if !matches!(root, "std" | "pkg") && !root.is_empty() && root != leaf {
        terms.push(root.to_lowercase());
    }
    terms
}

#[cfg(test)]
#[path = "module_paths_tests.rs"]
mod tests;
