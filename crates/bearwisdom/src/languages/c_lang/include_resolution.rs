// =============================================================================
// c_lang/include_resolution — place one `#include` spelling on an indexed file
// =============================================================================

/// The delimiter form of an include spelling. A quoted include searches the
/// including file's directory before the include paths; an angled include
/// searches the include paths only. A spelling stored without delimiters
/// carries no search-order evidence and places only on a unique supplied
/// external file, so a project header can never be guessed for it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum IncludeForm {
    Quoted,
    Angled,
    Undelimited,
}

fn split_form(include_specifier: &str) -> (IncludeForm, &str) {
    let trimmed = include_specifier.trim();
    if let Some(inner) = trimmed.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        (IncludeForm::Angled, inner)
    } else if let Some(inner) = trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        (IncludeForm::Quoted, inner)
    } else {
        (IncludeForm::Undelimited, trimmed)
    }
}

/// Resolve `include_specifier` as written in `source_file` to exactly one
/// indexed file of the same language, or `None` when no single file is the
/// unambiguous target.
///
/// Quoted form: the including file's directory first, then a unique
/// suffix match among project files, then among supplied external files.
/// Angled form: a unique suffix match among external files first, then among
/// project files (project include directories are ordinary `-I` roots).
pub(super) fn resolve(
    source_file: &str,
    include_specifier: &str,
    indexed_files: &[(&str, &str)],
) -> Option<String> {
    let source_language = indexed_files
        .iter()
        .find_map(|(path, language)| (*path == source_file).then_some(*language))?;
    let source = normalize(source_file);
    let (form, raw_spec) = split_form(include_specifier);
    let spec = normalize(raw_spec);
    if spec.is_empty() {
        return None;
    }
    let same_language: Vec<&str> = indexed_files
        .iter()
        .filter(|(path, language)| {
            same_include_family(language, source_language) && normalize(path) != source
        })
        .map(|(path, _)| *path)
        .collect();

    let explicit_relative = raw_spec
        .replace('\\', "/")
        .split('/')
        .next()
        .is_some_and(|part| matches!(part, "." | ".."));
    let source_dir = source.rsplit_once('/').map_or("", |(dir, _)| dir);
    if explicit_relative || form == IncludeForm::Quoted {
        let relative = normalize(&format!("{source_dir}/{spec}"));
        let exact: Vec<&str> = same_language
            .iter()
            .copied()
            .filter(|path| normalize(path) == relative)
            .collect();
        if exact.len() == 1 {
            return Some(exact[0].to_string());
        }
        if explicit_relative {
            return None;
        }
    }

    let internal = unique_suffix_match(&same_language, &spec, Some(false));
    let external = unique_suffix_match(&same_language, &spec, Some(true));
    match form {
        IncludeForm::Quoted => internal.or(external),
        IncludeForm::Angled => external.or(internal),
        // No search-order evidence: only a spelling that is unambiguous across
        // every indexed file, and supplied externally, is placed.
        IncludeForm::Undelimited => {
            unique_suffix_match(&same_language, &spec, None).filter(|path| path.starts_with("ext:"))
        }
    }
    .map(str::to_string)
}

/// C and C++ translation units share one header space: a `.h` the detector
/// tags as C++ is still what a C source includes.
fn same_include_family(language: &str, source_language: &str) -> bool {
    use crate::languages::LanguagePlugin;
    let family = super::CLangPlugin.language_ids();
    language == source_language
        || (family.contains(&language) && family.contains(&source_language))
}

/// The single file among `paths` whose normalized path equals `spec` or ends
/// with `/spec`; `external` restricts the candidates to supplied (`ext:`) or
/// project files, `None` considers both.
fn unique_suffix_match<'a>(
    paths: &[&'a str],
    spec: &str,
    external: Option<bool>,
) -> Option<&'a str> {
    let suffix = format!("/{spec}");
    let mut matches = paths.iter().copied().filter(|path| {
        external.is_none_or(|external| path.starts_with("ext:") == external) && {
            let normalized = normalize(path);
            normalized == spec || normalized.ends_with(&suffix)
        }
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn normalize(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|prior| *prior != "..") => {
                parts.pop();
            }
            ".." => parts.push(part),
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

#[cfg(test)]
#[path = "include_resolution_tests.rs"]
mod tests;
