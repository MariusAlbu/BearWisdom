// =============================================================================
// engine/module_scheme — URI-style scheme prefixes on module specifiers
//
// `node:assert/strict`, `sass:math`, `jsr:@std/path` — a single-colon scheme
// names a platform namespace in front of the module the specifier means. A
// double colon is a qualified-path separator (`crate::db`), never a scheme.
// =============================================================================

/// The specifier behind a single-colon scheme prefix, or `None` when the
/// specifier carries no scheme. `node:assert/strict` → `assert/strict`.
pub(crate) fn strip_scheme_prefix(spec: &str) -> Option<&str> {
    let ix = spec.find(':')?;
    if ix == 0 || spec.as_bytes().get(ix + 1) == Some(&b':') {
        return None;
    }
    let scheme = &spec[..ix];
    let valid = scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    if !valid {
        return None;
    }
    let rest = &spec[ix + 1..];
    (!rest.is_empty()).then_some(rest)
}

/// The bare Node builtin name behind a well-formed `node:` specifier.
///
/// Node's `node:` scheme is an alias for the builtin module namespace, so
/// `node:path` and `node:fs/promises` can be compared with declaration files
/// indexed under the corresponding bare paths. This deliberately accepts only
/// identifier-like path segments: a malformed traversal or another URI scheme
/// must keep its raw spelling and cannot acquire Node-builtin authority.
pub(crate) fn node_builtin_module_alias(spec: &str) -> Option<&str> {
    let alias = spec.strip_prefix("node:")?;
    if alias.is_empty()
        || alias.split('/').any(|segment| {
            segment.is_empty()
                || matches!(segment, "." | "..")
                || !segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
    {
        return None;
    }
    Some(alias)
}

#[cfg(test)]
#[path = "module_scheme_tests.rs"]
mod tests;
