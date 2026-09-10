// =============================================================================
// ecosystem/npm/module_specifier — npm and DefinitelyTyped specifier policy
//
// The resolver consumes ordered prefix candidates and a directory-match
// decision. npm owns the package-scope, @types, deep-import, and scheme rules
// that produce those generic values.
// =============================================================================

use crate::type_checker::profile::language_profile::{
    ModuleSpecifierClass, SourceModulePathPolicy,
};

/// Re-export file-path rules shared by JavaScript-family language profiles.
/// The generic re-export walker consumes only this neutral callback pair.
pub(crate) const SOURCE_MODULE_PATH_POLICY: SourceModulePathPolicy = SourceModulePathPolicy {
    classify_specifier: classify_module_specifier,
    relative_candidate_paths: relative_reexport_candidate_paths,
    bare_module_matches_file: never_matches_bare_module,
    external_import_match_terms,
};

/// JavaScript-family module spelling. Keep Windows drive paths with relative
/// source paths; all remaining non-empty forms are package/module specifiers.
pub(crate) fn classify_module_specifier(specifier: &str) -> ModuleSpecifierClass {
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

/// Candidate files for an extension-less JavaScript-family relative source.
/// Keep the literal base too: a source spelling may already include its
/// extension, or another adapter may have produced a concrete file path.
pub(crate) fn relative_reexport_candidate_paths(base: &str) -> Vec<String> {
    const EXTENSIONS: &[&str] = &[
        "ts", "tsx", "js", "jsx", "mjs", "mts", "cts", "cjs", "svelte", "astro", "vue",
    ];
    let mut out = Vec::with_capacity(EXTENSIONS.len() * 2 + 1);
    out.push(base.to_string());
    for extension in EXTENSIONS {
        out.push(format!("{base}.{extension}"));
    }
    for extension in EXTENSIONS {
        out.push(format!("{base}/index.{extension}"));
    }
    out
}

fn never_matches_bare_module(_file_path: &str, _source_module: &str) -> bool {
    false
}

fn external_import_match_terms(module: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let leaf = module.rsplit('/').next().unwrap_or(module);
    if !leaf.is_empty() {
        terms.push(leaf.to_lowercase());
    }
    let root = module.split('/').next().unwrap_or(module);
    if !root.is_empty() && root != leaf {
        terms.push(root.to_lowercase());
    }
    terms
}

/// Ordered qname prefixes for a JavaScript-family module specifier.
pub(crate) fn module_prefix_candidates(module: &str) -> Vec<String> {
    let mut out = vec![module.to_string()];
    if !is_bare_module_specifier(module) {
        return out;
    }

    if let Some((scheme, stripped)) = split_scheme(module) {
        append_unique(&mut out, module_prefix_candidates(stripped));
        append_unique(
            &mut out,
            module_prefix_candidates(&format!("{scheme}/{stripped}")),
        );
        return out;
    }

    if !module.starts_with("@types/") {
        if let Some(rest) = module.strip_prefix('@') {
            if let Some((scope, package)) = rest.split_once('/') {
                if !scope.is_empty() && !package.is_empty() {
                    out.push(format!("@types/{scope}__{package}"));
                }
            }
        } else {
            out.push(format!("@types/{module}"));
        }
    }

    let mut path = module;
    while let Some((parent, _)) = path.rsplit_once('/') {
        if parent.starts_with('@') && !parent.contains('/') {
            break;
        }
        path = parent;
        out.push(path.to_string());
    }
    out
}

/// Bare package specifiers must bind through exact npm qname evidence. A
/// same-named project directory is unrelated package evidence.
pub(crate) fn declines_directory_match(module: &str) -> bool {
    is_bare_module_specifier(module)
}

/// Package-entry key for an npm external virtual path. Scoped package grammar
/// belongs here, so callers only receive the canonical bare package key.
pub(crate) fn package_entry_key(path: &str) -> Option<String> {
    let package_path = path.strip_prefix("ext:ts:")?;
    npm_package_root(package_path).map(str::to_string)
}

fn npm_package_root(specifier: &str) -> Option<&str> {
    if specifier.starts_with('@') {
        let scope_end = specifier.find('/')?;
        let package_end = specifier[scope_end + 1..]
            .find('/')
            .map_or(specifier.len(), |offset| scope_end + 1 + offset);
        (package_end > scope_end + 1).then(|| &specifier[..package_end])
    } else {
        specifier
            .split('/')
            .next()
            .filter(|package| !package.is_empty())
    }
}

fn append_unique(out: &mut Vec<String>, candidates: Vec<String>) {
    for candidate in candidates {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
}

fn is_bare_module_specifier(specifier: &str) -> bool {
    SOURCE_MODULE_PATH_POLICY.is_bare(specifier)
}

fn split_scheme(specifier: &str) -> Option<(&str, &str)> {
    let colon = specifier.find(':')?;
    let scheme = &specifier[..colon];
    let path = &specifier[colon + 1..];
    if scheme.is_empty()
        || path.is_empty()
        || !scheme.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || (index > 0 && (byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')))
        })
    {
        return None;
    }
    Some((scheme, path))
}

#[cfg(test)]
#[path = "module_specifier_tests.rs"]
mod tests;
