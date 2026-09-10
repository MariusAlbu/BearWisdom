//! NuGet dependency spelling for CLR languages.

use rustc_hash::FxHashSet;

use crate::ecosystem::manifest::ManifestKind;

pub(crate) fn matches(
    kind: ManifestKind,
    names: &FxHashSet<String>,
    language: &str,
    spec: &str,
) -> bool {
    kind == ManifestKind::NuGet
        && matches!(language, "csharp" | "fsharp" | "vbnet")
        && names.iter().any(|name| name.eq_ignore_ascii_case(spec))
}
