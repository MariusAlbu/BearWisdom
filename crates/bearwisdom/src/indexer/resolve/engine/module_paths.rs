//! Module-location ingestion boundary. Exact paths only; never suffix candidates.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct PathRules {
    pub extensions: Vec<String>,
    pub substitutions: Vec<(String, Vec<String>)>,
    pub directory_entry: String,
}

pub(super) fn normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let absolute = path.starts_with('/');
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    format!("{}{}", if absolute { "/" } else { "" }, parts.join("/"))
}

pub(super) fn relative_base(source: &str, spec: &str) -> Option<String> {
    if !(spec.starts_with("./") || spec.starts_with("../")) {
        return None;
    }
    let source = normalize(source);
    let directory = source
        .rsplit_once('/')
        .map(|(directory, _)| directory)
        .unwrap_or("");
    Some(normalize(&if directory.is_empty() {
        spec.to_owned()
    } else {
        format!("{directory}/{spec}")
    }))
}

pub(super) fn find<T>(
    base: &str,
    rules: &PathRules,
    mut exact: impl FnMut(&str) -> Option<T>,
) -> Option<T> {
    for (suffix, substitutions) in &rules.substitutions {
        if let Some(stem) = base.strip_suffix(suffix) {
            return substitutions
                .iter()
                .find_map(|extension| exact(&format!("{stem}{extension}")));
        }
    }
    if let Some(found) = exact(base) {
        return Some(found);
    }
    for extension in &rules.extensions {
        if let Some(found) = exact(&format!("{base}{extension}")) {
            return Some(found);
        }
    }
    for extension in &rules.extensions {
        if let Some(found) = exact(&format!("{base}/{}{extension}", rules.directory_entry)) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
#[path = "module_paths_tests.rs"]
mod tests;
