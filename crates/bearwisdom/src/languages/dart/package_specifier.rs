//! Dart package URI and external-path package spelling.

fn owns(language: &str) -> bool {
    language == "dart"
}

fn root(specifier: &str) -> Option<String> {
    specifier
        .split('/')
        .next()
        .filter(|root| !root.is_empty())
        .map(str::to_string)
}

pub(crate) fn package_root(language: &str, specifier: &str) -> Option<String> {
    if !owns(language) {
        return None;
    }
    let package_path = specifier
        .strip_prefix("package:")
        .or_else(|| specifier.strip_prefix("dart:"))
        .unwrap_or(specifier);
    root(package_path)
}

/// The library file a Dart URI names, by basename stem: the last `/`
/// segment with its extension dropped (`package:a/src/b.dart` → `b`,
/// `../widgets.dart` → `widgets`, `dart:async` → `async`).
pub(crate) fn specifier_file_stem(language: &str, specifier: &str) -> Option<String> {
    if !owns(language) {
        return None;
    }
    let path = specifier
        .strip_prefix("package:")
        .or_else(|| specifier.strip_prefix("dart:"))
        .unwrap_or(specifier);
    let basename = path.rsplit('/').next().unwrap_or(path);
    let stem = basename.rsplit_once('.').map_or(basename, |(stem, _)| stem);
    (!stem.is_empty()).then(|| stem.to_string())
}

pub(crate) fn external_file_under_module(
    language: &str,
    file_path: &str,
    module_root: &str,
) -> Option<bool> {
    if !owns(language) {
        return None;
    }
    let Some(module_path) = file_path
        .strip_prefix("ext:dart:")
        .or_else(|| file_path.strip_prefix("ext:flutter-sdk:"))
    else {
        return Some(false);
    };
    Some(
        module_path == module_root
            || module_path
                .strip_prefix(module_root)
                .is_some_and(|suffix| suffix.starts_with('/')),
    )
}

pub(crate) fn external_package_key(language: &str, path: &str) -> Option<String> {
    owns(language)
        .then(|| external_package_key_from_path(path))
        .flatten()
}

pub(crate) fn external_package_key_from_path(path: &str) -> Option<String> {
    let package_path = path
        .strip_prefix("ext:dart:")
        .or_else(|| path.strip_prefix("ext:flutter-sdk:"))?;
    root(package_path)
}

pub(crate) fn external_package_matches_import(
    language: &str,
    path: &str,
    import_root: &str,
) -> Option<bool> {
    owns(language).then(|| external_package_key_from_path(path).as_deref() == Some(import_root))
}

#[cfg(test)]
mod tests {
    use super::{external_package_key, package_root, specifier_file_stem};

    #[test]
    fn dart_uri_stem_is_the_library_file_basename() {
        assert_eq!(
            specifier_file_stem("dart", "package:core_client/src/client.dart"),
            Some("client".into())
        );
        assert_eq!(
            specifier_file_stem("dart", "../widgets.dart"),
            Some("widgets".into())
        );
        assert_eq!(
            specifier_file_stem("dart", "dart:async"),
            Some("async".into())
        );
        assert_eq!(specifier_file_stem("typescript", "./widgets"), None);
    }

    #[test]
    fn dart_package_uri_matches_external_package_identity() {
        assert_eq!(
            package_root("dart", "package:flutter/material.dart"),
            Some("flutter".into())
        );
        assert_eq!(
            external_package_key("dart", "ext:dart:flutter/lib/material.dart"),
            Some("flutter".into())
        );
        assert_eq!(
            external_package_key("dart", "ext:flutter-sdk:flutter/src/widgets.dart"),
            Some("flutter".into())
        );
    }
}
