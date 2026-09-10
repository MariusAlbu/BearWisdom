//! PyPI distribution-name spelling for Python import roots.

use rustc_hash::FxHashSet;

use crate::ecosystem::manifest::ManifestKind;

pub(crate) fn matches(
    kind: ManifestKind,
    names: &FxHashSet<String>,
    language: &str,
    spec: &str,
) -> bool {
    matches!(
        kind,
        ManifestKind::PyProject | ManifestKind::PipRequirements
    ) && language == "python"
        && names.iter().any(|name| pypi_name(name) == pypi_name(spec))
}

fn pypi_name(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}
