use super::ERLANG_PROFILE;
use crate::type_checker::profile::language_profile::{NameNormalization, NormSpec};

#[test]
fn erlang_profile_identity() {
    assert_eq!(ERLANG_PROFILE.id, "erlang");
    assert_eq!(ERLANG_PROFILE.qname_separator, ":");
}

/// Apply the profile's NormSpec sigil-pair the way `normalize_name` does:
/// strip the matching `(prefix, suffix)` wrapper when the name both starts
/// with `prefix` and ends with `suffix`.
fn strip_quote_pair(spec: &NormSpec, s: &str) -> String {
    for (prefix, suffix) in spec.strip_sigils {
        if let Some(inner) = s.strip_prefix(*prefix).and_then(|i| i.strip_suffix(*suffix)) {
            return inner.to_string();
        }
    }
    s.to_string()
}

#[test]
fn erlang_quoted_atom_normalizes_to_bare_name() {
    let spec = match ERLANG_PROFILE.name_normalization {
        NameNormalization::Spec(spec) => spec,
        NameNormalization::None => panic!("erlang must normalize quoted atoms"),
    };
    // A quoted atom binds against the bare stored name.
    assert_eq!(strip_quote_pair(&spec, "'P_basic'"), "P_basic");
    // A bare name and an unquoted-but-apostrophe-bearing form are untouched —
    // the wrapper only strips when BOTH ends carry the quote.
    assert_eq!(strip_quote_pair(&spec, "P_basic"), "P_basic");
    assert_eq!(strip_quote_pair(&spec, "'half"), "'half");
}
