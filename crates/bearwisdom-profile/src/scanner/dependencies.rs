use crate::types::DetectedPackageManager;
use std::path::Path;

/// Detect which package managers are active in `root` for the given languages.
///
/// For each `PmDescriptor`:
/// - `has_lock_file` — the lock file exists in `root`.
/// - `deps_installed` — if `deps_dir` is set, it exists in `root`.
pub fn detect_package_managers(root: &Path, language_ids: &[String]) -> Vec<DetectedPackageManager> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for id in language_ids {
        let Some(lang) = crate::registry::find_language(id) else { continue };

        for pm in lang.package_managers {
            // Deduplicate by (language_id, pm_name) pair — JS/TS share npm.
            let key = format!("{}:{}", lang.id, pm.name);
            if !seen.insert(key) {
                continue;
            }

            let has_lock_file = pm.lock_file
                .map(|lf| root.join(lf).exists())
                .unwrap_or(false);

            let deps_installed = pm.deps_dir
                .map(|dd| root.join(dd).exists())
                .unwrap_or(true); // if no deps_dir declared, assume installed (e.g. cargo)

            // Only surface a PM if it has a positive signal (lock file or no lock file
            // defined but entry-point files were already confirmed by the caller).
            if has_lock_file || pm.lock_file.is_none() {
                results.push(DetectedPackageManager {
                    language_id: lang.id.to_owned(),
                    name: pm.name.to_owned(),
                    has_lock_file,
                    deps_installed,
                });
            }
        }
    }

    results
}
