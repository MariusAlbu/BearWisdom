//! Cargo crate spelling for Rust import roots.

use rustc_hash::FxHashSet;

use crate::ecosystem::manifest::ManifestKind;

pub(crate) fn matches(
    kind: ManifestKind,
    names: &FxHashSet<String>,
    language: &str,
    spec: &str,
) -> bool {
    kind == ManifestKind::Cargo
        && language == "rust"
        && names
            .iter()
            .any(|name| cargo_name(name) == cargo_name(spec))
}

fn cargo_name(name: &str) -> String {
    name.to_ascii_lowercase().replace('-', "_")
}
