// indexer/module_resolution/ruby_mod.rs — Ruby require resolver
//
// Resolution rules:
//   `require_relative "X"` — module field carries `./X` (or `../X`).
//      Resolve relative to the importing file's directory; look for
//      `<resolved>.rb` in the indexed file set.
//   `require "X"` — bare gem-style path (`sidekiq/api`, `net/http`).
//      For project-internal gems, the file lives at `lib/<X>.rb`.
//      Try `lib/<X>.rb` and `<X>.rb` against the indexed files.
//      Returns None for stdlib / external gem paths — those are
//      classified as external by the resolver, not by this layer.

use super::ModuleResolver;

pub struct RubyModuleResolver;

const LANGUAGES: &[&str] = &["ruby"];

impl ModuleResolver for RubyModuleResolver {
    fn language_ids(&self) -> &[&str] {
        LANGUAGES
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        if specifier.is_empty() {
            return None;
        }

        if specifier.starts_with('.') {
            return resolve_relative(specifier, importing_file, file_paths);
        }

        // Bare require: try `lib/<spec>.rb` first (the canonical gem
        // load path), then `<spec>.rb` at root. Match against the
        // suffix of any indexed file path.
        let lib_candidate = format!("lib/{specifier}.rb");
        if let Some(hit) = find_suffix(&lib_candidate, file_paths) {
            return Some(hit);
        }
        let root_candidate = format!("{specifier}.rb");
        find_suffix(&root_candidate, file_paths)
    }
}

/// Find a file whose normalized path equals `candidate` or ends with `/<candidate>`.
fn find_suffix(candidate: &str, file_paths: &[&str]) -> Option<String> {
    let suffix = format!("/{candidate}");
    for &p in file_paths {
        let norm = p.replace('\\', "/");
        if norm == candidate || norm.ends_with(&suffix) {
            return Some(p.to_string());
        }
    }
    None
}

fn resolve_relative(spec: &str, importing: &str, file_paths: &[&str]) -> Option<String> {
    let importing_norm = importing.replace('\\', "/");
    let dir = importing_norm
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("");
    let combined = if dir.is_empty() {
        spec.to_string()
    } else {
        format!("{dir}/{spec}")
    };
    let normalized = normalize_path(&combined)?;
    let candidate = format!("{normalized}.rb");
    for &p in file_paths {
        let norm = p.replace('\\', "/");
        if norm == candidate {
            return Some(p.to_string());
        }
    }
    None
}

fn normalize_path(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
#[path = "ruby_mod_tests.rs"]
mod tests;
