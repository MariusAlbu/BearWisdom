//! Scope-aware type-name lookup over canonical index qualified names.

use super::Symbol;
use crate::indexer::resolve::engine::support::{index_qname_parent, join_index_qname};
use crate::type_checker::profile::language_profile::LanguageProfile;
use std::collections::BTreeMap;

pub(crate) fn resolve_type_name_in_scope(
    raw: &str,
    scope_path: Option<&str>,
    by_qname: &BTreeMap<String, Symbol>,
    profile: &LanguageProfile,
) -> String {
    let source_is_qualified = profile.is_qualified_name(raw);
    let raw = profile.index_qname_from_source(raw);
    // `raw` now carries the canonical index representation. Its dotted form
    // is storage syntax, not a source-language separator.
    if source_is_qualified || index_qname_parent(&raw).is_some() {
        return raw;
    }
    let Some(scope) = scope_path else {
        return raw;
    };
    // Scope paths and symbol keys use the canonical index separator. A source
    // spelling is normalized first so profile separators never leak into lookup.
    let mut current = profile.index_qname_from_source(scope);
    loop {
        let candidate = if current.is_empty() {
            raw.clone()
        } else {
            join_index_qname(&current, &raw)
        };
        if by_qname.contains_key(&candidate) {
            return candidate;
        }
        if current.is_empty() {
            break;
        }
        current = index_qname_parent(&current).unwrap_or("").to_string();
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

    #[test]
    fn resolves_a_non_dot_source_separator_against_canonical_index_keys() {
        let mut names = BTreeMap::new();
        names.insert(
            "pkg.Outer.Thing".into(),
            Symbol {
                id: 1,
                name: "Thing".into(),
                qualified_name: "pkg.Outer.Thing".into(),
                kind: "class".into(),
                visibility: None,
                file_path: std::sync::Arc::from("test"),
                scope_path: None,
                package_id: None,
                signature: None,
            },
        );
        let profile = LanguageProfile {
            qname_separator: "::",
            ..DEFAULT_PROFILE
        };
        assert_eq!(
            resolve_type_name_in_scope("Thing", Some("pkg::Outer"), &names, &profile),
            "pkg.Outer.Thing"
        );
        assert_eq!(
            resolve_type_name_in_scope("pkg::Outer::Thing", None, &names, &profile),
            "pkg.Outer.Thing"
        );
    }
}
