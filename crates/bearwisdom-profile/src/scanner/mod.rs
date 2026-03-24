pub mod dependencies;
pub mod entry_points;
pub mod environment;
pub mod frameworks;
pub mod languages;
pub mod monorepo;
pub mod sdks;

use crate::types::{LanguageStats, ProjectProfile, RestoreTrigger, ScanOptions};
use std::collections::HashMap;
use std::path::Path;

/// Result of a full project scan, including the file manifest.
pub struct ScanResult {
    pub profile: ProjectProfile,
    pub file_manifest: Vec<crate::types::ScannedFile>,
}

/// Scan `root` and return a `ProjectProfile`.
///
/// This is the primary public entry point for the crate.
/// Delegates to [`scan_with_manifest`] and discards the file manifest.
pub fn scan(root: &Path, options: ScanOptions) -> ProjectProfile {
    scan_with_manifest(root, options).profile
}

/// Scan `root` and return both the `ProjectProfile` and the full file manifest.
///
/// The file manifest is the sorted list of all indexable source files found
/// during phase 1. Callers that need to index files (e.g. `bearwisdom`)
/// can use this to avoid a second directory walk.
pub fn scan_with_manifest(root: &Path, options: ScanOptions) -> ScanResult {
    tracing::debug!("scanning project at {}", root.display());

    // --- Phase 1: Count files by language (also produces the file manifest) ---
    let (counts, file_manifest) = languages::count_files_with_manifest(root);

    // Collect language ids that have at least one file.
    let mut language_ids: Vec<String> = counts.keys().cloned().collect();
    language_ids.sort();

    // --- Phase 2: Entry points per language ---
    let mut lang_stats: Vec<LanguageStats> = language_ids
        .iter()
        .filter_map(|id| {
            let lang = crate::registry::find_language(id)?;
            let file_count = *counts.get(id).unwrap_or(&0);
            let entry_points = entry_points::find_entry_points(
                root,
                lang.entry_point_files,
                options.max_depth,
            );
            Some(LanguageStats {
                language_id: id.clone(),
                display_name: lang.display_name.to_owned(),
                file_count,
                entry_points,
            })
        })
        .collect();

    // Sort by file count descending.
    lang_stats.sort_by(|a, b| b.file_count.cmp(&a.file_count));

    // Only keep languages that have confirmed entry points OR ≥5 files.
    // This filters out "yaml because one .yml config was found" from polluting
    // the primary language list while keeping YAML when the project has many.
    let primary_language_ids: Vec<String> = lang_stats
        .iter()
        .filter(|s| !s.entry_points.is_empty() || s.file_count >= 5)
        .map(|s| s.language_id.clone())
        .collect();

    // --- Phase 3: SDKs ---
    let sdks = sdks::check_sdks(root, &primary_language_ids, options.check_sdks);

    // --- Phase 4: Package managers ---
    let package_managers = dependencies::detect_package_managers(root, &primary_language_ids);

    // --- Phase 5: Test frameworks ---
    let test_frameworks = frameworks::detect_test_frameworks(root, &primary_language_ids);

    // --- Phase 6: Monorepo ---
    let monorepo = monorepo::detect_monorepo(root);

    // --- Phase 7: Environment ---
    let environment = environment::detect_environment(root);

    // --- Phase 8: Restore steps ---
    let restore_steps = collect_restore_steps(root, &primary_language_ids, &package_managers);

    let profile = ProjectProfile {
        root: root.to_string_lossy().into_owned(),
        languages: lang_stats,
        sdks,
        package_managers,
        test_frameworks,
        monorepo,
        environment,
        restore_steps,
        meta: HashMap::new(),
    };

    ScanResult { profile, file_manifest }
}

fn collect_restore_steps(
    root: &Path,
    language_ids: &[String],
    detected_pms: &[crate::types::DetectedPackageManager],
) -> Vec<String> {
    let mut steps = Vec::new();

    // Check missing env file first — universal.
    if root.join(".env.example").exists() && !root.join(".env").exists() {
        steps.push("Copy .env.example to .env and fill in secrets".to_owned());
    }

    for id in language_ids {
        let Some(lang) = crate::registry::find_language(id) else { continue };

        for step in lang.restore_steps {
            let triggered = match step.trigger {
                RestoreTrigger::DirMissing => {
                    let dir = root.join(step.watch_path);
                    !dir.is_dir()
                }
                RestoreTrigger::FileMissing => {
                    let f = root.join(step.watch_path);
                    !f.exists()
                }
                RestoreTrigger::FileExists => {
                    root.join(step.watch_path).exists()
                }
                RestoreTrigger::SdkVersionMismatch => {
                    // Evaluated by the sdks module; we skip it here.
                    false
                }
            };

            if triggered {
                steps.push(format!("{}: {}", step.title, step.description));
            }
        }
    }

    // Also flag missing deps dirs from detected package managers.
    for pm in detected_pms {
        if pm.has_lock_file && !pm.deps_installed {
            steps.push(format!(
                "Run `{}` — lock file present but deps directory missing",
                pm.name
            ));
        }
    }

    steps
}
