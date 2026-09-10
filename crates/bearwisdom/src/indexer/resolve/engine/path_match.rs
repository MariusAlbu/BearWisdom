// =============================================================================
// engine/path_match — file-path vs module-specifier comparison primitives
//
// Every rung that decides "does this symbol's file plausibly belong to that
// import's module?" goes through these. Two string families meet here and must
// not be conflated: REAL file paths (extension-bearing, `/`-separated, maybe
// externally indexed) and MODULE specifiers (relative paths, bare package
// names, or qualified names). Language profiles adapt source spellings before
// these primitives compare them.
// =============================================================================

/// A bare module specifier names a package, not a project-relative path.
/// Rejects specifiers that start with `.`, `/`, or a Windows drive letter
/// (`C:/`). Callers that scope a specifier to a workspace package's symbol
/// set must first rule out a relative/absolute path.
pub(crate) fn is_bare_module_specifier(spec: &str) -> bool {
    !(spec.starts_with('.')
        || spec.starts_with('/')
        || (spec.len() >= 2 && spec.as_bytes()[1] == b':'))
}

/// A specifier is relative — and therefore project-internal — when it starts
/// with `.`, `/`, or is a Windows drive path.
pub(crate) fn is_relative_specifier(s: &str) -> bool {
    s.starts_with('.') || s.starts_with('/') || (s.len() >= 2 && s.as_bytes()[1] == b':')
}

/// The full directory portion of a file path (everything before the final
/// segment). Path separators are normalized to `/`. Returns `None` for a bare
/// filename. For `schema/users/model.prisma` returns `Some("schema/users")`.
pub(crate) fn parent_dir(file_path: &str) -> Option<String> {
    let normalized = file_path.replace('\\', "/");
    normalized.rsplit_once('/').map(|(dir, _)| dir.to_string())
}

/// The file's basename-stem equals the module (case-insensitive on both
/// inputs). A basename with no extension matches whole. Does NOT consider
/// directory segments.
pub(crate) fn basename_stem_matches(file_path_lower: &str, module_lower: &str) -> bool {
    if module_lower.is_empty() {
        return false;
    }
    let normalized = file_path_lower.replace('\\', "/");
    let Some(basename) = normalized.rsplit('/').next() else {
        return false;
    };
    match basename.rsplit_once('.') {
        Some((stem, _ext)) => stem == module_lower,
        None => basename == module_lower,
    }
}

/// File path's basename stem or any path segment matches the module
/// (case-insensitive on both inputs). External `ext:<lang>:<pkg>` paths match on
/// the trailing colon-delimited component.
pub(crate) fn path_stem_matches(file_path_lower: &str, module_lower: &str) -> bool {
    if basename_stem_matches(file_path_lower, module_lower) {
        return true;
    }
    let normalized = file_path_lower.replace('\\', "/");
    normalized.split('/').any(|seg| {
        seg == module_lower
            || seg
                .split(':')
                .next_back()
                .map_or(false, |tail| tail == module_lower)
    })
}

/// Trim the file extension off a REAL path: everything after the last `.` in
/// the final `/`-segment. Module specifiers must not go through this — a
/// dotted qualified name (`java.util.Map`) has no extension and would lose
/// its last segment; use `trim_source_extension` for those.
pub(crate) fn trim_path_extension(path: &str) -> &str {
    match (path.rfind('/'), path.rfind('.')) {
        (Some(slash), Some(dot)) if dot > slash => &path[..dot],
        (None, Some(dot)) => &path[..dot],
        _ => path,
    }
}

/// Match a symbol's file path against an import module specifier. Returns `true`
/// when the path plausibly names the same file as `module`:
/// - Stem-suffix: the path's extension-stripped form ends with the module's
///   extension-stripped, `./`/`../`-trimmed form (covers relative imports).
/// - Dot-to-slash: same after replacing `.` with `/` in the module (covers
///   dotted package imports like `posthog.models` and dotted FQNs like
///   `java.util.Map`).
/// - Segment-bounded run: the slash-form of the module appears as a
///   `/`-bounded contiguous run inside the path (covers `__init__.py`
///   re-exports and deep package paths the stem-suffix check misses).
pub(crate) fn file_path_matches_module(
    file_path: &str,
    module: &str,
    profile: &crate::type_checker::profile::language_profile::LanguageProfile,
) -> bool {
    if module.is_empty() {
        return false;
    }
    let normalized = file_path.replace('\\', "/");
    let adapted = profile.module_path_match(module);
    if adapted.authority
        == crate::type_checker::profile::language_profile::ModuleMatchAuthority::Reject
    {
        return false;
    }
    if adapted.required_file_prefix.is_some_and(|prefix| {
        !normalized.starts_with(&prefix.replace('\\', "/"))
    }) {
        return false;
    }
    file_path_matches_normalized_module(
        &normalized,
        &adapted.module_path,
        adapted.compound_extensions,
    )
}

fn file_path_matches_normalized_module(
    normalized: &str,
    module: &str,
    compound_extensions: &[&str],
) -> bool {
    let path_like = module.starts_with('.') || module.starts_with('/') || module.contains('/');
    let module = module.trim_start_matches("./").trim_start_matches("../");
    if path_suffix_matches(normalized, module) {
        return true;
    }
    let cleaned = if path_like {
        trim_known_or_final_extension(module, compound_extensions)
    } else {
        module
    };
    let stem = trim_known_or_final_extension(normalized, compound_extensions);
    if stem.ends_with(cleaned) || stem.ends_with(&cleaned.replace('.', "/")) {
        return true;
    }
    let dotted = cleaned.replace('.', "/");
    if dotted.is_empty() {
        return false;
    }
    path_contains_segment_run(&normalized, &dotted)
}

fn trim_known_or_final_extension<'a>(path: &'a str, compound_extensions: &[&str]) -> &'a str {
    for suffix in compound_extensions {
        if let Some(stem) = path.strip_suffix(suffix) {
            return stem;
        }
    }
    trim_path_extension(path)
}

fn path_suffix_matches(path: &str, suffix: &str) -> bool {
    path.strip_suffix(suffix)
        .is_some_and(|head| head.is_empty() || head.ends_with('/'))
}

/// `true` when `run` appears in `path` as a `/`-bounded contiguous segment run.
pub(crate) fn path_contains_segment_run(path: &str, run: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = path[from..].find(run) {
        let start = from + rel;
        let end = start + run.len();
        let left_ok = start == 0 || path.as_bytes()[start - 1] == b'/';
        let right_ok = end == path.len() || path.as_bytes()[end] == b'/';
        if left_ok && right_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

#[cfg(test)]
#[path = "path_match_tests.rs"]
mod tests;
