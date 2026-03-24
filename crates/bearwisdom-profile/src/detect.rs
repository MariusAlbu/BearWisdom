use crate::registry::LANGUAGES;
use crate::types::LanguageDescriptor;
use std::path::Path;

/// Detect the language for a given file path.
///
/// Detection priority:
/// 1. Exact filename match (e.g. "Dockerfile", "Makefile").
/// 2. File extension match (e.g. ".rs", ".ts").
///
/// Returns `None` for unknown file types (images, binaries, etc.).
pub fn detect_language(path: &Path) -> Option<&'static LanguageDescriptor> {
    // 1. Exact filename match — highest priority.
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        for lang in LANGUAGES {
            if lang.filenames.contains(&name) {
                return Some(lang);
            }
        }
    }

    // 2. Extension match.
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_with_dot = format!(".{ext}");
        for lang in LANGUAGES {
            if lang.file_extensions.contains(&ext_with_dot.as_str()) {
                return Some(lang);
            }
        }
    }

    None
}
