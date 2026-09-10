// =============================================================================
// rust_lang/module_paths — Rust source module spelling to physical files
// =============================================================================

use crate::indexer::resolve::engine::support::{parent_dir, path_contains_segment_run};
use crate::type_checker::profile::import_specs::{
    ModuleMatchAuthority, ModulePathMatch, ModuleSpecifierClass, SourceModulePathPolicy,
};

const RUST_SOURCE_MODULE_PATH_POLICY: SourceModulePathPolicy = SourceModulePathPolicy {
    classify_specifier: classify_module_specifier,
    relative_candidate_paths: rust_relative_candidate_paths,
    bare_module_matches_file: rust_module_matches_file,
    external_import_match_terms: rust_external_import_match_terms,
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

/// Source-module policy consumed by generic resolver rules. Rust owns its
/// crate-directory underscore spelling; the engine sees only match evidence.
pub(crate) fn module_path_match(module: &str) -> ModulePathMatch {
    ModulePathMatch {
        module_path: module.to_string(),
        // Cargo externals retain their crate root in the Rust-owned virtual
        // path envelope. Supplying that complete physical prefix keeps the
        // generic matcher free of virtual-path delimiter grammar.
        path_variants: rust_external_path_variants(module),
        required_file_prefix: None,
        compound_extensions: &[],
        // Rust module spellings are matched only by this adapter's path
        // evidence. In particular, external Cargo crate roots are not npm
        // packages and must not fall through to an unrelated ecosystem.
        authority: ModuleMatchAuthority::Authoritative,
        source_module_path_policy: RUST_SOURCE_MODULE_PATH_POLICY,
    }
}

fn rust_external_path_variants(module: &str) -> Vec<String> {
    let crate_root = module.split("::").next().unwrap_or(module);
    (!crate_root.is_empty())
        .then(|| format!("ext:rust:{crate_root}"))
        .into_iter()
        .collect()
}

pub(crate) fn module_prefix_candidates(module: &str) -> Vec<String> {
    vec![module.to_string()]
}

pub(crate) fn does_not_decline_directory_match(_module: &str) -> bool {
    false
}

fn rust_relative_candidate_paths(base: &str) -> Vec<String> {
    vec![
        base.to_string(),
        format!("{base}.rs"),
        format!("{base}/mod.rs"),
    ]
}

fn rust_external_import_match_terms(module: &str) -> Vec<String> {
    module
        .split("::")
        .filter(|term| !term.is_empty())
        .last()
        .map(|term| vec![term.to_lowercase()])
        .unwrap_or_default()
}

fn rust_module_matches_file(file_path: &str, source_module: &str) -> bool {
    let run = source_module
        .replace("::", "/")
        .replace('.', "/")
        .trim_start_matches("./")
        .trim_start_matches("../")
        .to_string();
    if run.is_empty() {
        return false;
    }
    let file = file_path.replace('\\', "/").replace('-', "_");
    let run = run.replace('-', "_");
    let module_file = file.strip_suffix(".rs").unwrap_or(&file);
    path_contains_segment_run(module_file, &run)
        // Rust package roots conventionally insert `src` between the crate
        // directory and the source module. That physical layout is Rust-owned
        // evidence, not a generic module-path fallback.
        || path_contains_segment_run(&module_file.replace("/src/", "/"), &run)
}

/// Canonical package-import spelling for Cargo package matching. Rust paths
/// use a language-owned qualification separator while manifest package names
/// use the resolver's path-shaped package key.
pub(crate) fn workspace_package_specifier(specifier: &str) -> Option<String> {
    specifier
        .contains("::")
        .then(|| specifier.replace("::", "/"))
}

/// Candidate module-root files for a Rust `use super::*` or
/// `use crate::x::*` wildcard. The generic resolver consumes only the returned
/// paths and preserves its usual unique-hit rule.
pub(crate) fn relative_wildcard_module_files(
    importing_file: &str,
    module: &str,
    target: &str,
) -> Vec<String> {
    if target.is_empty() || target.contains('.') || target.contains("::") {
        return Vec::new();
    }
    let file = importing_file.replace('\\', "/");
    let head = module.split("::").next().unwrap_or(module);
    match head {
        "super" => {
            let Some(parent) = module_parent_dir(&file) else {
                return Vec::new();
            };
            let rest: Vec<&str> = module.split("::").skip(1).collect();
            module_roots(&parent, &rest)
        }
        "crate" => {
            let src_dir = crate_src_dir(&file);
            let rest: Vec<&str> = module.split("::").skip(1).collect();
            if rest.is_empty() {
                vec![format!("{src_dir}/lib.rs"), format!("{src_dir}/main.rs")]
            } else {
                module_roots(&src_dir, &rest)
            }
        }
        _ => Vec::new(),
    }
}

fn module_parent_dir(file: &str) -> Option<String> {
    let dir = parent_dir(file)?;
    let basename = file.rsplit('/').next().unwrap_or(file);
    let stem = basename.rsplit_once('.').map_or(basename, |(s, _)| s);
    if matches!(stem, "mod" | "lib" | "main") {
        parent_dir(&dir)
    } else {
        Some(dir)
    }
}

fn crate_src_dir(file: &str) -> String {
    let mut acc: Vec<&str> = Vec::new();
    let mut last_src: Option<usize> = None;
    for (i, seg) in file.split('/').enumerate() {
        acc.push(seg);
        if seg == "src" {
            last_src = Some(i);
        }
    }
    match last_src {
        Some(i) => acc[..=i].join("/"),
        None => parent_dir(file).unwrap_or_default(),
    }
}

fn module_roots(base_dir: &str, segments: &[&str]) -> Vec<String> {
    if segments.is_empty() {
        let leaf = base_dir.rsplit('/').next().unwrap_or(base_dir);
        let sibling = parent_dir(base_dir)
            .map(|p| format!("{p}/{leaf}.rs"))
            .unwrap_or_else(|| format!("{base_dir}.rs"));
        return vec![format!("{base_dir}/mod.rs"), sibling];
    }
    let nested = segments.join("/");
    vec![
        format!("{base_dir}/{nested}.rs"),
        format!("{base_dir}/{nested}/mod.rs"),
    ]
}

#[cfg(test)]
#[path = "module_paths_tests.rs"]
mod tests;
