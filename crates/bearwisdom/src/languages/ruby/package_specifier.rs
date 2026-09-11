//! RubyGems package spelling for resolver package evidence.

const LANGUAGES: &[&str] = &["ruby", "rbi", "rbs"];

fn owns(language: &str) -> bool {
    LANGUAGES.contains(&language)
}

fn root(specifier: &str) -> Option<String> {
    specifier
        .split('/')
        .next()
        .filter(|root| !root.is_empty())
        .map(str::to_string)
}

pub(crate) fn package_root(language: &str, specifier: &str) -> Option<String> {
    owns(language).then(|| root(specifier)).flatten()
}

pub(crate) fn external_file_under_module(
    language: &str,
    file_path: &str,
    module_root: &str,
) -> Option<bool> {
    if !owns(language) {
        return None;
    }
    let Some(module_path) = file_path.strip_prefix("ext:ruby:") else {
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
    root(path.strip_prefix("ext:ruby:")?)
}

/// Ruby's `require "aws/..."` family admits gems whose package name begins
/// with that require root followed by a hyphen (`aws-sdk-s3`). The boundary is
/// RubyGems policy, kept here with the package and virtual-path spelling.
pub(crate) fn external_package_matches_import(
    language: &str,
    path: &str,
    import_root: &str,
) -> Option<bool> {
    if !owns(language) {
        return None;
    }
    Some(external_package_key_from_path(path).is_some_and(|package| {
        package == import_root
            || package
                .strip_prefix(import_root)
                .is_some_and(|suffix| suffix.starts_with('-'))
    }))
}

#[cfg(test)]
mod tests {
    use super::{external_package_key, external_package_key_from_path, package_root};

    #[test]
    fn ruby_gem_paths_keep_the_gem_name() {
        assert_eq!(package_root("ruby", "aws/sdk/s3"), Some("aws".into()));
        assert_eq!(
            external_package_key("ruby", "ext:ruby:aws-sdk-s3/lib/client.rb"),
            Some("aws-sdk-s3".into())
        );
        assert_eq!(
            external_package_key_from_path("ext:ruby:aws-sdk-s3/lib/client.rb"),
            Some("aws-sdk-s3".into())
        );
    }
}
