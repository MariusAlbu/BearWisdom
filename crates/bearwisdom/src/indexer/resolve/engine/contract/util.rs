// =============================================================================
// indexer/resolve/engine/util.rs — pure utility functions for the resolve layer
//
// Small predicates and helpers with no dependency on SymbolIndex or the
// resolve loop's state: scope-chain construction from a qualified name,
// type-kind classification, and import specifiers.
// =============================================================================

pub(crate) fn is_type_like_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "interface"
            | "enum"
            | "type_alias"
            | "namespace"
            | "record"
            | "trait"
            | "protocol"
            | "object"
            | "mixin"
            | "extension"
    )
}

pub(crate) fn common_prefix_len(a: &str, b: &str, separator: &str) -> usize {
    a.split(separator)
        .zip(b.split(separator))
        .take_while(|(x, y)| x == y)
        .count()
}

// ---------------------------------------------------------------------------
// Path / name normalisation for the template-include import resolver
// ---------------------------------------------------------------------------

/// Resolve `.`/`..` segments lexically without touching the filesystem, so the
/// candidate paths match the forward-slash form stored on indexed symbols.
///
/// Stack-based: a `..` only pops a preceding normal/`.` segment. A leading
/// `..` (escaping above the path's own root) is kept rather than popped past
/// the root, so `../x` stays `../x`.
pub fn lexical_normalize(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut stack: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                let pop_ok = matches!(
                    stack.last(),
                    Some(Component::Normal(_)) | Some(Component::CurDir)
                );
                if pop_ok {
                    stack.pop();
                } else {
                    stack.push(comp);
                }
            }
            Component::CurDir => {}
            other => stack.push(other),
        }
    }
    stack.iter().collect()
}

/// Kebab-case a camelCase / PascalCase identifier: `UserCard` → `user-card`,
/// `userCard` → `user-card`. Returns `None` when the input has no uppercase
/// letter (nothing to convert). A `-` is inserted only at a lower→upper
/// boundary, so an all-caps run (`HTTPServer`) doesn't gain interior dashes
/// before each capital.
pub fn camel_to_kebab(s: &str) -> Option<String> {
    let has_upper = s.chars().any(|c| c.is_ascii_uppercase());
    if !has_upper {
        return None;
    }
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_lower = false;
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            if prev_lower {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else {
            out.push(ch);
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Helpers for building RefContext
// ---------------------------------------------------------------------------

/// Build the scope chain from a symbol's scope_path.
///
/// scope_path = "A.B.C" → ["A.B.C", "A.B", "A"]
pub fn build_scope_chain(
    scope_path: Option<&str>,
    profile: &crate::type_checker::profile::language_profile::LanguageProfile,
) -> Vec<String> {
    let Some(path) = scope_path else {
        return Vec::new();
    };
    if path.is_empty() {
        return Vec::new();
    }

    let mut chain = Vec::new();
    let mut current = profile.index_qname_from_source(path);
    chain.push(current.clone());

    while let Some(parent) = crate::indexer::resolve::engine::support::index_qname_parent(&current)
    {
        current.truncate(parent.len());
        chain.push(current.clone());
    }

    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};

    #[test]
    fn canonical_scope_chain_does_not_follow_source_separator() {
        let profile = LanguageProfile {
            qname_separator: "::",
            ..DEFAULT_PROFILE
        };
        assert_eq!(
            build_scope_chain(Some("pkg.Outer.Inner"), &profile),
            vec!["pkg.Outer.Inner", "pkg.Outer", "pkg"]
        );
    }
}
