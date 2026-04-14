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

        // 2. Compound extension match — `.blade.php`, `.html.twig`,
        // `.d.ts`, etc. Done BEFORE single-extension fallback so a
        // `welcome.blade.php` is detected as Blade, not PHP. We pick
        // the LONGEST compound match (sorted by suffix length descending)
        // so `.blade.php` (10) wins over `.php` (4) on the same file.
        let lname = name.to_ascii_lowercase();
        let mut best: Option<(&'static LanguageDescriptor, usize)> = None;
        for lang in LANGUAGES {
            for ext in lang.file_extensions {
                // Only consider compound extensions here (those with
                // more than one dot inside) — single-dot extensions
                // are handled by the single-extension match below.
                if ext.matches('.').count() < 2 { continue; }
                let lext = ext.to_ascii_lowercase();
                if lname.ends_with(&lext) {
                    let len = lext.len();
                    if best.map_or(true, |(_, l)| len > l) {
                        best = Some((lang, len));
                    }
                }
            }
        }
        if let Some((lang, _)) = best {
            return Some(lang);
        }
    }

    // 3. Single-extension match.
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
