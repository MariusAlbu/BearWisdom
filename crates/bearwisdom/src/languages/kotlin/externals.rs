// =============================================================================
// kotlin/externals.rs — Kotlin/Android external dependency locator
//
// This locator chains two discovery strategies:
//
//   1. Java Maven sources jars — delegates to `JavaExternalsLocator` which
//      walks `~/.m2/repository` for declared Gradle/Maven dependencies.
//      This covers all JVM libraries (Retrofit, OkHttp, Hilt, Room, etc.).
//
//   2. Android SDK android.jar — reads `$ANDROID_HOME/platforms/android-<N>/
//      android.jar` for the highest installed API level and extracts it via
//      `extract_java_sources_jar`. This provides symbols for Activity, Context,
//      View, Fragment, Intent, Bundle, etc. which aren't in any Maven repo.
//
// Both strategies gracefully skip when their prerequisites are absent (no
// ANDROID_HOME, no Maven local repo) — only a debug log is emitted.
// =============================================================================

use crate::indexer::externals::{
    ExternalDepRoot, ExternalSourceLocator, JavaExternalsLocator, extract_java_sources_jar,
    is_cache_stale,
};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// JVM dependency symbols are indexed by the JavaExternalsLocator from Maven
/// source jars. No hardcoded externals needed.
pub(crate) const EXTERNALS: &[&str] = &[];

// ---------------------------------------------------------------------------
// KotlinExternalsLocator
// ---------------------------------------------------------------------------

/// External dependency locator for Kotlin/Android projects.
///
/// Chains Maven sources jars (via `JavaExternalsLocator`) with Android SDK
/// platform jars (via `$ANDROID_HOME`).
pub struct KotlinExternalsLocator;

impl ExternalSourceLocator for KotlinExternalsLocator {
    fn ecosystem(&self) -> &'static str {
        "kotlin"
    }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        let mut roots = JavaExternalsLocator.locate_roots(project_root);
        roots.extend(discover_android_sdk_roots());
        roots
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        // Both Maven source dirs and Android SDK extracted dirs contain .java files;
        // the java walker handles both.
        JavaExternalsLocator.walk_root(dep)
    }
}

// ---------------------------------------------------------------------------
// Android SDK discovery
// ---------------------------------------------------------------------------

/// Locate the Android SDK via `$ANDROID_HOME` (or `$ANDROID_SDK_ROOT` as a
/// fallback), find the highest installed API level under `platforms/`, extract
/// `android.jar` to a persistent cache, and return it as an `ExternalDepRoot`.
///
/// Returns an empty vec when:
///   - Neither env var is set
///   - The `platforms/` directory doesn't exist or is empty
///   - `android.jar` is missing for the selected API level
///   - Extraction fails
///
/// All failure modes are silent (debug log only) — never a hard error.
pub fn discover_android_sdk_roots() -> Vec<ExternalDepRoot> {
    let Some(sdk_root) = android_home() else {
        debug!("ANDROID_HOME not set — skipping Android SDK externals");
        return Vec::new();
    };

    let platforms_dir = sdk_root.join("platforms");
    if !platforms_dir.is_dir() {
        debug!(
            "Android SDK platforms dir not found at {} — skipping",
            platforms_dir.display()
        );
        return Vec::new();
    }

    // Pick the highest installed API level: directories named `android-<N>`.
    let api_level = match highest_api_level(&platforms_dir) {
        Some(v) => v,
        None => {
            debug!("No android-<N> platform directories found in {} — skipping", platforms_dir.display());
            return Vec::new();
        }
    };

    let platform_dir = platforms_dir.join(format!("android-{api_level}"));
    let jar_path = platform_dir.join("android.jar");
    if !jar_path.is_file() {
        debug!(
            "android.jar not found at {} — skipping",
            jar_path.display()
        );
        return Vec::new();
    }

    // Cache extracted .java stubs alongside the SDK to avoid re-extracting.
    let cache_base = sdk_root.join("bearwisdom-android-cache");
    let cache_dir = cache_base.join(format!("android-{api_level}"));

    if !cache_dir.exists() || is_cache_stale(&jar_path, &cache_dir) {
        debug!("Extracting android.jar (API {api_level}) to {}", cache_dir.display());
        if let Err(e) = extract_java_sources_jar(&jar_path, &cache_dir) {
            debug!(
                "Failed to extract android.jar at {}: {e}",
                jar_path.display()
            );
            return Vec::new();
        }
    }

    debug!("Android SDK API {api_level} externals registered from {}", cache_dir.display());
    vec![ExternalDepRoot {
        module_path: format!("android-sdk:{api_level}"),
        version: api_level.to_string(),
        root: cache_dir,
        ecosystem: "kotlin",
        package_id: None,
    }]
}

/// Read `$ANDROID_HOME` (preferred) or `$ANDROID_SDK_ROOT` (legacy).
fn android_home() -> Option<PathBuf> {
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(val) = std::env::var(var) {
            let p = PathBuf::from(val);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// Scan `platforms_dir` for `android-<N>` subdirectories and return the
/// highest N found, or `None` if there are none.
fn highest_api_level(platforms_dir: &Path) -> Option<u32> {
    let entries = std::fs::read_dir(platforms_dir).ok()?;
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_str()?;
            let n: u32 = s.strip_prefix("android-")?.parse().ok()?;
            if e.path().is_dir() { Some(n) } else { None }
        })
        .max()
}
