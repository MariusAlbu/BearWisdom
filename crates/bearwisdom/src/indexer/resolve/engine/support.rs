// =============================================================================
// indexer/resolve/engine/support — helpers shared by multiple rules
//
// Only code that genuinely repeats across rule files lives here. A helper used
// by a single rule is copied inline into that rule's file instead, so each rule
// reads top-to-bottom without chasing this module. Everything here is a pure
// function of its inputs.
// =============================================================================

use std::borrow::Cow;

use crate::type_checker::profile::language_profile::{NameNormalization, NormSpec};

/// `true` when `kind` names a type a `this`/`self` keyword or an inherited
/// member can attach to — a class-like declaration, not a namespace, function,
/// or value.
pub(crate) fn is_type_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "interface"
            | "enum"
            | "trait"
            | "object"
            | "record"
            | "protocol"
            | "actor"
            | "mixin"
            | "annotation"
    )
}

/// `true` when `qualified_name` reads as `module_path` (slash / colon /
/// dot-separated) prefix followed by `.` and one or more segments.
pub(crate) fn qname_under_module(qualified_name: &str, module_path: &str) -> bool {
    let dotted = module_path.replace("::", ".").replace('/', ".");
    if dotted.is_empty() {
        return false;
    }
    let needle = format!("{dotted}.");
    qualified_name.starts_with(&needle) || qualified_name == dotted
}

/// Stricter form of `qname_under_module`: candidate must sit DIRECTLY under the
/// module — exactly one segment deeper. `Assertions.assertTrue` matches
/// `Assertions`; `Assertions.Nested.foo` does not.
pub(crate) fn qname_directly_under(qualified_name: &str, module_path: &str) -> bool {
    let dotted = module_path.replace("::", ".").replace('/', ".");
    if dotted.is_empty() {
        return false;
    }
    let needle = format!("{dotted}.");
    let Some(rest) = qualified_name.strip_prefix(needle.as_str()) else {
        return false;
    };
    !rest.contains('.')
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

/// Trim a source-file extension off a module/path string for stem comparison.
pub(crate) fn trim_source_extension(path: &str) -> &str {
    path.trim_end_matches(".svelte")
        .trim_end_matches(".vue")
        .trim_end_matches(".tsx")
        .trim_end_matches(".jsx")
        .trim_end_matches(".mts")
        .trim_end_matches(".cts")
        .trim_end_matches(".ts")
        .trim_end_matches(".js")
        .trim_end_matches(".cs")
        .trim_end_matches(".cljc")
        .trim_end_matches(".cljs")
        .trim_end_matches(".clj")
        .trim_end_matches(".astro")
        .trim_end_matches(".mdx")
}

/// Strip a leading `{kw}.` from `target` when `kw` is one of `self_keywords`
/// (Python `self.method` → `method`). Only the first matching keyword strips,
/// and only when followed by `.`. An empty `self_keywords` slice returns
/// `target` unchanged.
pub(crate) fn strip_self_keyword<'t>(target: &'t str, self_keywords: &[&str]) -> &'t str {
    for kw in self_keywords {
        if let Some(rest) = target.strip_prefix(kw) {
            if let Some(after) = rest.strip_prefix('.') {
                return after;
            }
        }
    }
    target
}

/// Normalize a name for the bare-name binding comparison. Applied identically to
/// a candidate's name and the ref's target before they are compared.
///
/// `NameNormalization::None` is the identity transform and borrows unchanged. A
/// `Spec` whose deltas are all off is also identity and borrows. Otherwise the
/// transform runs in a fixed order: strip a wrapping sigil pair, strip the first
/// matching leading prefix, remove the configured characters anywhere, then fold
/// ASCII case.
pub(crate) fn normalize_name(norm: NameNormalization, s: &str) -> Cow<'_, str> {
    let spec = match norm {
        NameNormalization::None => return Cow::Borrowed(s),
        NameNormalization::Spec(spec) => spec,
    };
    if is_identity_spec(&spec) {
        return Cow::Borrowed(s);
    }

    let mut cur = s;

    // 1. Sigil wrapper: when the name both starts with `prefix` and ends with
    //    `suffix`, drop both. The first matching pair wins.
    for (prefix, suffix) in spec.strip_sigils {
        if let Some(inner) = cur.strip_prefix(*prefix) {
            if let Some(inner) = inner.strip_suffix(*suffix) {
                cur = inner;
                break;
            }
        }
    }

    // 2. Leading prefix: drop the first declared prefix that matches. A prefix
    //    runs before the case-fold step, so when the spec folds case the prefix
    //    match folds too; a case-sensitive spec keeps the exact byte match.
    for prefix in spec.strip_prefixes {
        let matched_len = if spec.case_insensitive {
            cur.get(..prefix.len())
                .filter(|head| head.eq_ignore_ascii_case(prefix))
                .map(|_| prefix.len())
        } else {
            cur.starts_with(*prefix).then_some(prefix.len())
        };
        if let Some(len) = matched_len {
            cur = &cur[len..];
            break;
        }
    }

    // 3 & 4. Remove the configured characters anywhere and fold case. Both need
    //        an owned buffer; build it once.
    let needs_char_strip = !spec.strip_chars.is_empty();
    if !needs_char_strip && !spec.case_insensitive {
        return Cow::Borrowed(cur);
    }
    let mut out = String::with_capacity(cur.len());
    for ch in cur.chars() {
        if needs_char_strip && spec.strip_chars.contains(&ch) {
            continue;
        }
        if spec.case_insensitive {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

/// A `NormSpec` whose every field is the default (no sigils, no prefixes, no
/// chars, case-sensitive) is the identity transform — `normalize_name` borrows
/// rather than allocating for it.
pub(crate) fn is_identity_spec(spec: &NormSpec) -> bool {
    !spec.case_insensitive
        && spec.strip_chars.is_empty()
        && spec.strip_prefixes.is_empty()
        && spec.strip_sigils.is_empty()
}

#[cfg(test)]
#[path = "support_tests.rs"]
mod tests;
