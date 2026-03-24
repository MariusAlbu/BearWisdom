use crate::types::DetectedTestFramework;
use std::path::Path;

/// Detect test frameworks for the given languages by checking config file presence
/// and optional content matches.
pub fn detect_test_frameworks(root: &Path, language_ids: &[String]) -> Vec<DetectedTestFramework> {
    let mut results = Vec::new();

    for id in language_ids {
        let Some(lang) = crate::registry::find_language(id) else { continue };

        for tf in lang.test_frameworks {
            if is_framework_present(root, tf) {
                results.push(DetectedTestFramework {
                    language_id: lang.id.to_owned(),
                    name: tf.name.to_owned(),
                    display_name: tf.display_name.to_owned(),
                    run_cmd: tf.run_cmd.bash.to_owned(),
                });
            }
        }
    }

    results
}

fn is_framework_present(root: &Path, tf: &crate::types::TfDescriptor) -> bool {
    for config_file in tf.config_files {
        // Config files may use glob-like names (e.g. "*.csproj"). Handle simple
        // exact matches and glob-less patterns here; full glob support is
        // intentionally out of scope for this crate.
        if config_file.contains('*') {
            // Walk one level for wildcard patterns.
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if glob_match(config_file, &name_str)
                        && check_content_match(root, &name_str, tf)
                    {
                        return true;
                    }
                }
            }
        } else {
            let candidate = root.join(config_file);
            if candidate.exists() && check_content_match(root, config_file, tf) {
                return true;
            }
        }
    }
    false
}

fn check_content_match(root: &Path, file: &str, tf: &crate::types::TfDescriptor) -> bool {
    match tf.config_content_match {
        None => true,
        Some(needle) => {
            let path = root.join(file);
            std::fs::read_to_string(&path)
                .map(|content| content.contains(needle))
                .unwrap_or(false)
        }
    }
}

/// Minimal glob matching: only `*` wildcard supported (matches any sequence of
/// non-separator chars). Handles the `*.csproj` style patterns used in descriptors.
fn glob_match(pattern: &str, name: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        pattern == name
    }
}
