//! C and C++ include-path resolution policy.

pub(super) fn resolve(
    source_file: &str,
    include_specifier: &str,
    indexed_files: &[(&str, &str)],
) -> Option<String> {
    let source_language = indexed_files
        .iter()
        .find_map(|(path, language)| (*path == source_file).then_some(*language))?;
    let source = normalize(source_file);
    let explicit_relative = include_specifier
        .replace('\\', "/")
        .split('/')
        .next()
        .is_some_and(|part| matches!(part, "." | ".."));
    let spec = normalize(include_specifier);
    if spec.is_empty() {
        return None;
    }

    // Include refs retain the path but not the quote/angle delimiter. Only an
    // explicit relative spelling may select a project header by directory;
    // otherwise a same-named project file could shadow a supplied header.
    if explicit_relative {
        let source_dir = source.rsplit_once('/').map_or("", |(dir, _)| dir);
        let relative = normalize(&format!("{source_dir}/{spec}"));
        let exact: Vec<&str> = indexed_files
            .iter()
            .filter(|(path, language)| {
                *language == source_language
                    && normalize(path) == relative
                    && normalize(path) != source
            })
            .map(|(path, _)| *path)
            .collect();
        return (exact.len() == 1).then(|| exact[0].to_string());
    }

    // Supplied headers retain an external virtual-path prefix. Match the
    // compiler-visible spelling at a path boundary and fail on ambiguity.
    let suffix = format!("/{spec}");
    let matches: Vec<&str> = indexed_files
        .iter()
        .filter(|(path, language)| {
            let normalized = normalize(path);
            *language == source_language
                && normalized != source
                && (normalized == spec || normalized.ends_with(&suffix))
        })
        .map(|(path, _)| *path)
        .collect();
    (matches.len() == 1 && matches[0].starts_with("ext:")).then(|| matches[0].to_string())
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
mod tests {
    use super::*;

    #[test]
    fn explicit_relative_include_prefers_the_source_directory() {
        let files = [
            ("src/main.c", "c"),
            ("src/local.h", "c"),
            ("ext:idx:/sdk/local.h", "c"),
        ];
        assert_eq!(
            resolve("src/main.c", "./local.h", &files).as_deref(),
            Some("src/local.h")
        );
    }

    #[test]
    fn bare_include_requires_one_external_match() {
        let files = [
            ("src/main.c", "c"),
            ("src/stdio.h", "c"),
            ("ext:idx:/sdk/include/stdio.h", "c"),
        ];
        assert_eq!(resolve("src/main.c", "stdio.h", &files), None);
    }
}
