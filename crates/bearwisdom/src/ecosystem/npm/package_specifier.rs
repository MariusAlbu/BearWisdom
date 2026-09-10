//! npm package-root and external virtual-path spelling.

const LANGUAGES: &[&str] = &[
    "typescript",
    "tsx",
    "javascript",
    "vue",
    "svelte",
    "angular",
    "astro",
    "scss",
];

fn owns(language: &str) -> bool {
    LANGUAGES.contains(&language)
}

pub(super) fn package_root(language: &str, specifier: &str) -> Option<String> {
    if !owns(language) {
        return None;
    }
    let root = if specifier.starts_with('@') {
        specifier
            .match_indices('/')
            .nth(1)
            .map_or(specifier, |(index, _)| &specifier[..index])
    } else {
        specifier.split('/').next().unwrap_or(specifier)
    };
    Some(root.to_string())
}

pub(super) fn external_file_under_module(
    language: &str,
    file_path: &str,
    module_root: &str,
) -> Option<bool> {
    if !owns(language) {
        return None;
    }
    let Some(module_path) = file_path.strip_prefix("ext:ts:") else {
        return Some(false);
    };
    Some(
        module_path == module_root
            || module_path
                .strip_prefix(module_root)
                .is_some_and(|suffix| suffix.starts_with('/')),
    )
}

pub(super) fn external_package_key(language: &str, path: &str) -> Option<String> {
    if !owns(language) {
        return None;
    }
    external_package_key_from_path(path)
}

pub(super) fn external_package_key_from_path(path: &str) -> Option<String> {
    let module = path.strip_prefix("ext:ts:")?;
    package_root("typescript", module)
}
