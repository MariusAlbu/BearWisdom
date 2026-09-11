//! Ecosystem-owned package-specifier and external-path policy.
//!
//! Resolver code supplies the active language and asks for a semantic package
//! operation. Adapters claim only the languages whose package grammar they own.

#[path = "npm/package_specifier.rs"]
mod npm;

type PackageRoot = fn(&str, &str) -> Option<String>;
type ExternalFileMatch = fn(&str, &str, &str) -> Option<bool>;
type ExternalPackageKey = fn(&str, &str) -> Option<String>;
type ExternalPackageKeyFromPath = fn(&str) -> Option<String>;
type ExternalPackageImportMatch = fn(&str, &str, &str) -> Option<bool>;

struct Adapter {
    package_root: PackageRoot,
    external_file_under_module: ExternalFileMatch,
    external_package_key: ExternalPackageKey,
    external_package_key_from_path: ExternalPackageKeyFromPath,
    external_package_matches_import: ExternalPackageImportMatch,
}

const ADAPTERS: &[Adapter] = &[
    Adapter {
        package_root: npm::package_root,
        external_file_under_module: npm::external_file_under_module,
        external_package_key: npm::external_package_key,
        external_package_key_from_path: npm::external_package_key_from_path,
        external_package_matches_import: npm::external_package_matches_import,
    },
    Adapter {
        package_root: crate::languages::ruby::package_specifier::package_root,
        external_file_under_module:
            crate::languages::ruby::package_specifier::external_file_under_module,
        external_package_key: crate::languages::ruby::package_specifier::external_package_key,
        external_package_key_from_path:
            crate::languages::ruby::package_specifier::external_package_key_from_path,
        external_package_matches_import:
            crate::languages::ruby::package_specifier::external_package_matches_import,
    },
    Adapter {
        package_root: crate::languages::dart::package_specifier::package_root,
        external_file_under_module:
            crate::languages::dart::package_specifier::external_file_under_module,
        external_package_key: crate::languages::dart::package_specifier::external_package_key,
        external_package_key_from_path:
            crate::languages::dart::package_specifier::external_package_key_from_path,
        external_package_matches_import:
            crate::languages::dart::package_specifier::external_package_matches_import,
    },
];

pub(crate) fn package_root(language: &str, specifier: &str) -> Option<String> {
    ADAPTERS
        .iter()
        .find_map(|adapter| (adapter.package_root)(language, specifier))
}

/// The import-side package root, when an ecosystem adapter owns this language.
pub(crate) fn import_package_root(language: &str, specifier: &str) -> Option<String> {
    package_root(language, specifier)
}

/// Semantic package key for an external virtual path. Both the virtual-path
/// spelling and scoped-package grammar stay in the owning ecosystem adapter.
pub(crate) fn external_package_key(language: &str, path: &str) -> Option<String> {
    ADAPTERS
        .iter()
        .find_map(|adapter| (adapter.external_package_key)(language, path))
}

/// Whether an external virtual path belongs to an imported package root. The
/// adapter owns both external-path grammar and any ecosystem-specific package
/// family relationship; unsupported languages decline rather than widening a
/// generic resolver match.
pub(crate) fn external_package_matches_import(
    language: &str,
    path: &str,
    import_root: &str,
) -> Option<bool> {
    ADAPTERS
        .iter()
        .find_map(|adapter| (adapter.external_package_matches_import)(language, path, import_root))
}

/// Semantic package key inferred from the adapter that owns this external
/// virtual path. Path-only callers do not select a language; virtual-path
/// spelling remains isolated in the corresponding adapter.
pub(crate) fn external_package_key_from_path(path: &str) -> Option<String> {
    ADAPTERS
        .iter()
        .find_map(|adapter| (adapter.external_package_key_from_path)(path))
}

pub(crate) fn external_file_under_module(
    language: &str,
    file_path: &str,
    module_root: &str,
) -> Option<bool> {
    ADAPTERS
        .iter()
        .find_map(|adapter| (adapter.external_file_under_module)(language, file_path, module_root))
}

/// Canonical workspace-package spelling for manifest/package matching. Each
/// adapter declines syntax it does not own; an unchanged string is already a
/// manifest-shaped package key.
pub(crate) fn workspace_package_specifier(specifier: &str) -> String {
    crate::languages::rust_lang::module_paths::workspace_package_specifier(specifier)
        .unwrap_or_else(|| specifier.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        external_file_under_module, external_package_key, external_package_key_from_path,
        external_package_matches_import, import_package_root, package_root,
    };

    #[test]
    fn dispatch_is_scoped_to_the_owning_ecosystem_language() {
        assert_eq!(
            package_root("typescript", "@scope/pkg/sub"),
            Some("@scope/pkg".to_string())
        );
        assert_eq!(package_root("go", "example.com/org/pkg"), None);
        assert_eq!(
            external_file_under_module(
                "typescript",
                "ext:ts:@scope/pkg/build/index.d.ts",
                "@scope/pkg"
            ),
            Some(true)
        );
        assert_eq!(
            external_file_under_module("go", "ext:go:example.com/org/pkg/file.go", "example.com"),
            None
        );
    }

    #[test]
    fn external_package_key_preserves_npm_scopes() {
        assert_eq!(import_package_root("go", "example.com/org/pkg"), None);
        assert_eq!(
            external_package_key("typescript", "ext:ts:@scope/pkg/build/index.d.ts"),
            Some("@scope/pkg".into())
        );
        assert_eq!(
            external_package_key_from_path("ext:ts:@scope/pkg/build/index.d.ts"),
            Some("@scope/pkg".into())
        );
    }

    #[test]
    fn package_match_dispatch_keeps_exact_and_family_rules_with_owners() {
        assert_eq!(
            external_package_matches_import("typescript", "ext:ts:react/index.d.ts", "react"),
            Some(true)
        );
        assert_eq!(
            external_package_matches_import("typescript", "ext:ts:react-dom/index.d.ts", "react"),
            Some(false),
            "npm package names are exact"
        );
        assert_eq!(
            external_package_matches_import(
                "dart",
                "ext:flutter-sdk:flutter/src/widgets.dart",
                "flutter"
            ),
            Some(true)
        );
        assert_eq!(
            external_package_matches_import(
                "dart",
                "ext:dart:flutter_tools/lib/src.dart",
                "flutter"
            ),
            Some(false),
            "Dart package names are exact"
        );
        assert_eq!(
            external_package_matches_import("ruby", "ext:ruby:aws-sdk-s3/lib/client.rb", "aws"),
            Some(true)
        );
        assert_eq!(
            external_package_matches_import("ruby", "ext:ruby:awsome/lib/client.rb", "aws"),
            Some(false),
            "Ruby family matching requires the adapter-owned hyphen boundary"
        );
    }
}
