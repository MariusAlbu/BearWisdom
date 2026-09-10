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

#[cfg(test)]
#[path = "module_scheme_tests.rs"]
mod tests;
