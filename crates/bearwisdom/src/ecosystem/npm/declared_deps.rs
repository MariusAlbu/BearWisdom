//! npm dependency spelling (`@scope/pkg/subpath`) for JS-family sources.

use rustc_hash::FxHashSet;

use crate::ecosystem::manifest::ManifestKind;

const LANGUAGES: &[&str] = &[
    "typescript",
    "tsx",
    "javascript",
    "jsx",
    "vue",
    "svelte",
    "angular",
    "astro",
    "scss",
];

pub(crate) fn matches(
    kind: ManifestKind,
    names: &FxHashSet<String>,
    language: &str,
    spec: &str,
) -> bool {
    kind == ManifestKind::Npm
        && LANGUAGES.contains(&language)
        && npm_package_root(spec).is_some_and(|name| names.contains(name))
}

fn npm_package_root(spec: &str) -> Option<&str> {
    if spec.starts_with('.') || spec.starts_with('/') {
        return None;
    }
    if spec.starts_with('@') {
        let second_slash = spec[1..].find('/')? + 1;
        let package_end = spec[second_slash + 1..]
            .find('/')
            .map(|offset| second_slash + 1 + offset)
            .unwrap_or(spec.len());
        return Some(&spec[..package_end]);
    }
    Some(spec.split('/').next().unwrap_or(spec))
}
