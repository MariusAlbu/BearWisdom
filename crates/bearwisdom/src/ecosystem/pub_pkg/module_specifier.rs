// =============================================================================
// ecosystem/pub_pkg/module_specifier — Dart Pub module-entry spelling
// =============================================================================

const EXTERNAL_PATH_PREFIX: &str = "ext:dart:";
pub(super) const PACKAGE_URI_PREFIX: &str = "package:";

/// The package root a Pub external library exposes to a bare module-entry
/// lookup. Pub owns both the `ext:dart:` envelope and this package spelling.
pub(crate) fn package_entry_key(path: &str) -> Option<String> {
    let library = path.strip_prefix(EXTERNAL_PATH_PREFIX)?;
    library
        .split('/')
        .next()
        .filter(|package| !package.is_empty())
        .map(str::to_string)
}

/// Exact package-URI keys contributed by one indexed Pub library.
pub(crate) fn entry_aliases(path: &str) -> Vec<String> {
    let Some(library) = path.strip_prefix(EXTERNAL_PATH_PREFIX) else {
        return Vec::new();
    };
    if !library.contains('/') {
        return Vec::new();
    }
    vec![format!("{PACKAGE_URI_PREFIX}{library}")]
}

/// Resolve a schemeless Pub export relative to its owning external library and
/// return the exact package-URI key stored for the destination library.
pub(crate) fn relative_entry_key(source_file: &str, specifier: &str) -> Option<String> {
    let source = source_file.strip_prefix(EXTERNAL_PATH_PREFIX)?;
    if specifier.contains(':') {
        return None;
    }
    let (package, source_library) = source.split_once('/')?;
    let source_dir = source_library.rsplit_once('/').map_or("", |(dir, _)| dir);
    let joined = if source_dir.is_empty() {
        specifier.to_string()
    } else {
        format!("{source_dir}/{specifier}")
    };
    let mut normalized = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                normalized.pop()?;
            }
            part => normalized.push(part),
        }
    }
    Some(format!(
        "{PACKAGE_URI_PREFIX}{package}/{}",
        normalized.join("/")
    ))
}

#[cfg(test)]
#[path = "module_specifier_tests.rs"]
mod tests;
