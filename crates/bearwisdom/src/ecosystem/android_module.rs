// ecosystem/android_module.rs — the Android modules a Gradle build declares.
//
// The Android Gradle Plugin is what turns a Gradle module into an Android
// module: it installs the `android { }` extension, consumes an
// `AndroidManifest.xml`, and compiles against a platform SDK. So the
// evidence this reader accepts is the plugin's application, not a mention of
// its namespace — a `classpath` entry or an `exclude group:` line naming
// `com.android.*` is a dependency-graph statement, not an opt-in.
//
// Two acceptance rules:
//   1. The build script applies a plugin id in AGP's publisher namespace,
//      written literally or reached through a version-catalog alias.
//   2. The build script declares the `android { }` extension AGP installs,
//      corroborated by a compile-SDK pin or the module's own
//      `AndroidManifest.xml`. This covers builds that apply AGP through a
//      convention plugin, whose own id is not in the namespace.

use std::path::{Path, PathBuf};

use super::android_compile_sdk::{compile_sdk_pin, load_version_catalogs, VersionCatalogs};
use super::manifest::gradle::{collect_gradle_build_files, collect_version_catalogs};
use super::manifest::gradle_build_root::gradle_build_root;
use super::manifest::gradle_plugins::{
    applied_plugin_ids, load_plugin_catalogs, strip_line_comment, PluginCatalogs,
};
use super::manifest::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

/// Gradle plugin-id namespace the Android Gradle Plugin publishes under —
/// `com.android.application`, `com.android.library`, `com.android.test`,
/// `com.android.dynamic-feature` and the rest of the family.
const AGP_NAMESPACE: &str = "com.android.";

/// The extension block AGP installs on a module it is applied to.
const ANDROID_EXTENSION: &str = "android";

/// Conventional filename declaring an Android component's manifest.
const ANDROID_MANIFEST_FILE: &str = "AndroidManifest.xml";

/// One Gradle module that builds an Android component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidModule {
    /// Directory holding the module's build script.
    pub package_dir: PathBuf,
    /// The build script itself.
    pub build_file: PathBuf,
    /// AGP-namespace plugin ids the script applies. Empty for a module
    /// recognized through the extension block alone.
    pub plugin_ids: Vec<String>,
    /// Platform API level the module compiles against, when it pins one.
    pub compile_sdk: Option<u32>,
}

/// The Android modules under `scan_root`. Version catalogs resolve against
/// the Gradle build `scan_root` belongs to, because a subproject reaches a
/// catalog its settings file declares, not one of its own.
pub fn scan_android_modules(workspace_root: &Path, scan_root: &Path) -> Vec<AndroidModule> {
    let catalogs = reachable_catalog_files(workspace_root, scan_root);
    let plugin_catalogs = load_plugin_catalogs(&catalogs);
    let version_catalogs = load_version_catalogs(&catalogs);

    let mut out = Vec::new();
    for build_file in collect_gradle_build_files(scan_root) {
        let Ok(content) = std::fs::read_to_string(&build_file) else {
            continue;
        };
        let package_dir = match build_file.parent() {
            Some(dir) => dir.to_path_buf(),
            None => continue,
        };
        if let Some(module) = classify(
            package_dir,
            build_file,
            &content,
            &plugin_catalogs,
            &version_catalogs,
        ) {
            out.push(module);
        }
    }
    out
}

/// The module `content` declares, or `None` when neither acceptance rule
/// holds.
fn classify(
    package_dir: PathBuf,
    build_file: PathBuf,
    content: &str,
    plugin_catalogs: &PluginCatalogs,
    version_catalogs: &VersionCatalogs,
) -> Option<AndroidModule> {
    let plugin_ids: Vec<String> = applied_plugin_ids(content, plugin_catalogs)
        .into_iter()
        .filter(|id| id.starts_with(AGP_NAMESPACE))
        .collect();
    let compile_sdk = compile_sdk_pin(content, version_catalogs);

    let accepted = !plugin_ids.is_empty()
        || (declares_android_extension(content)
            && (compile_sdk.is_some() || has_android_manifest(&package_dir)));
    if !accepted {
        return None;
    }
    Some(AndroidModule {
        package_dir,
        build_file,
        plugin_ids,
        compile_sdk,
    })
}

/// Whether `content` opens the `android { }` extension block. The accessor
/// must stand alone so `androidComponents { }` and `optionalAndroid { }` —
/// a different extension and a convention-plugin helper — do not match.
fn declares_android_extension(content: &str) -> bool {
    content.lines().any(|raw| {
        let line = strip_line_comment(raw).trim();
        line.strip_prefix(ANDROID_EXTENSION)
            .is_some_and(|rest| rest.trim_start().starts_with('{'))
    })
}

/// Whether the module ships its own component manifest, at the module root
/// (the flat layout) or under one of its source sets.
fn has_android_manifest(package_dir: &Path) -> bool {
    if package_dir.join(ANDROID_MANIFEST_FILE).is_file() {
        return true;
    }
    let Ok(source_sets) = std::fs::read_dir(package_dir.join("src")) else {
        return false;
    };
    source_sets
        .flatten()
        .any(|entry| entry.path().join(ANDROID_MANIFEST_FILE).is_file())
}

/// The catalog files `scan_root` can reach: the ones its build root declares
/// plus any it declares itself, deduplicated by accessor and path.
fn reachable_catalog_files(workspace_root: &Path, scan_root: &Path) -> Vec<(String, PathBuf)> {
    let build_root = gradle_build_root(workspace_root, scan_root);
    let mut out = collect_version_catalogs(&build_root);
    if build_root.as_path() == scan_root {
        return out;
    }
    for entry in collect_version_catalogs(scan_root) {
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Manifest reader
// ---------------------------------------------------------------------------

/// Surfaces one manifest entry per Android module so the android-sdk
/// ecosystem activates for the packages that declare one — and only those.
pub struct AndroidModuleManifest;

impl ManifestReader for AndroidModuleManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::AndroidModule
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() {
            return None;
        }
        let mut data = ManifestData::default();
        for entry in &entries {
            data.dependencies
                .extend(entry.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        scan_android_modules(project_root, project_root)
            .into_iter()
            .map(|module| ReaderEntry {
                package_dir: module.package_dir,
                manifest_path: module.build_file,
                data: ManifestData {
                    dependencies: module.plugin_ids.into_iter().collect(),
                    ..ManifestData::default()
                },
                name: None,
            })
            .collect()
    }
}

/// The distinct platform API levels a module set pins, ascending.
pub fn pinned_api_levels(modules: &[AndroidModule]) -> Vec<u32> {
    let mut levels: Vec<u32> = modules.iter().filter_map(|m| m.compile_sdk).collect();
    levels.sort_unstable();
    levels.dedup();
    levels
}

#[cfg(test)]
#[path = "android_module_tests.rs"]
mod tests;
