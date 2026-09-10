// =============================================================================
// ecosystem/npm/module_specifier — npm and DefinitelyTyped specifier policy
//
// The resolver consumes ordered prefix candidates and a directory-match
// decision. npm owns the package-scope, @types, deep-import, and scheme rules
// that produce those generic values.
// =============================================================================

/// Ordered qname prefixes for a JavaScript-family module specifier.
pub(crate) fn module_prefix_candidates(module: &str) -> Vec<String> {
    let mut out = vec![module.to_string()];
    if !is_bare_module_specifier(module) {
        return out;
    }

    if let Some((scheme, stripped)) = split_scheme(module) {
        append_unique(&mut out, module_prefix_candidates(stripped));
        append_unique(
            &mut out,
            module_prefix_candidates(&format!("{scheme}/{stripped}")),
        );
        return out;
    }

    if !module.starts_with("@types/") {
        if let Some(rest) = module.strip_prefix('@') {
            if let Some((scope, package)) = rest.split_once('/') {
                if !scope.is_empty() && !package.is_empty() {
                    out.push(format!("@types/{scope}__{package}"));
                }
            }
        } else {
            out.push(format!("@types/{module}"));
        }
    }

    let mut path = module;
    while let Some((parent, _)) = path.rsplit_once('/') {
        if parent.starts_with('@') && !parent.contains('/') {
            break;
        }
        path = parent;
        out.push(path.to_string());
    }
    out
}

/// Bare package specifiers must bind through exact npm qname evidence. A
/// same-named project directory is unrelated package evidence.
pub(crate) fn declines_directory_match(module: &str) -> bool {
    is_bare_module_specifier(module)
}

fn append_unique(out: &mut Vec<String>, candidates: Vec<String>) {
    for candidate in candidates {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
}

fn is_bare_module_specifier(specifier: &str) -> bool {
    !(specifier.starts_with('.')
        || specifier.starts_with('/')
        || (specifier.len() >= 2 && specifier.as_bytes()[1] == b':'))
}

fn split_scheme(specifier: &str) -> Option<(&str, &str)> {
    let colon = specifier.find(':')?;
    let scheme = &specifier[..colon];
    let path = &specifier[colon + 1..];
    if scheme.is_empty()
        || path.is_empty()
        || !scheme
            .bytes()
            .enumerate()
            .all(|(index, byte)| {
                byte.is_ascii_alphabetic()
                    || (index > 0 && (byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')))
            })
    {
        return None;
    }
    Some((scheme, path))
}

#[cfg(test)]
#[path = "module_specifier_tests.rs"]
mod tests;
