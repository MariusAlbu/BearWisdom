// =============================================================================
// ecosystem/android_sdk.rs — Android platform SDK (stdlib for Kotlin + Java)
//
// Serves the `sources/android-<N>` tree of an SDK install, or the `.java`
// stubs extracted from `platforms/android-<N>/android.jar` when sources are
// not installed.
//
// The platform is chosen by the compile-SDK pin the project's own Android
// modules declare. A project that declares no Android module gets no SDK:
// the platform redefines `java.*`/`javax.*`, so walking it into a plain JVM
// build gives every core type a second declaration.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{debug, warn};

use super::android_module::{pinned_api_levels, scan_android_modules, AndroidModule};
use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{
    extract_java_sources_jar, is_cache_stale, ExternalDepRoot, ExternalSourceLocator,
};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("android-sdk");
const LEGACY_ECOSYSTEM_TAG: &str = "android-sdk";
const LANGUAGES: &[&str] = &["kotlin", "java"];

/// Activation clauses: an Android module is a Gradle module, so the JVM
/// package ecosystem carries its dependency coordinates, and the
/// `AndroidModule` manifest kind carries the module declarations themselves.
const ACTIVATION: &[EcosystemActivation] = &[
    EcosystemActivation::TransitiveOn(super::maven::ID),
    EcosystemActivation::ManifestMatch,
];

pub struct AndroidSdkEcosystem;

impl Ecosystem for AndroidSdkEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::All(ACTIVATION)
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_android_sdk_roots(ctx.project_root, ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }

    fn supports_reachability(&self) -> bool {
        true
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[crate::ecosystem::externals::ExternalDepRoot],
    ) -> crate::ecosystem::symbol_index::SymbolLocationIndex {
        super::maven::build_maven_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for AndroidSdkEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }
    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_android_sdk_roots(project_root, project_root)
    }
    fn locate_roots_for_package(
        &self,
        workspace_root: &Path,
        package_abs_path: &Path,
        package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        // The package's own build script names the platform it compiles
        // against; catalogs still resolve against the workspace's build root.
        let mut roots = discover_android_sdk_roots(workspace_root, package_abs_path);
        for root in &mut roots {
            root.package_id = Some(package_id);
        }
        roots
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

/// Dep roots for the platforms the Android modules under `scan_root` compile
/// against. Empty when the tree declares no Android module, when no SDK is
/// installed, or when the declared platform is absent from the install.
pub(crate) fn discover_android_sdk_roots(
    workspace_root: &Path,
    scan_root: &Path,
) -> Vec<ExternalDepRoot> {
    let modules = scan_android_modules(workspace_root, scan_root);
    if modules.is_empty() {
        return Vec::new();
    }
    let Some(sdk_root) = android_sdk_home() else {
        warn!(
            "android-sdk: {} declares {} Android module(s) but no SDK install was found; \
             set BEARWISDOM_ANDROID_SDK or ANDROID_HOME",
            scan_root.display(),
            modules.len()
        );
        return Vec::new();
    };

    let levels = requested_api_levels(&sdk_root, &modules, scan_root);
    levels
        .into_iter()
        .filter_map(|level| platform_root(&sdk_root, level))
        .collect()
}

/// The platform levels to serve: every level the modules pin, or the newest
/// installed one when they pin none.
fn requested_api_levels(sdk_root: &Path, modules: &[AndroidModule], scan_root: &Path) -> Vec<u32> {
    let pinned = pinned_api_levels(modules);
    if !pinned.is_empty() {
        return pinned;
    }
    let Some(newest) = newest_installed_level(sdk_root) else {
        return Vec::new();
    };
    warn!(
        "android-sdk: {} declares an Android module but pins no compileSdk; \
         falling back to the newest installed platform android-{}",
        scan_root.display(),
        newest
    );
    vec![newest]
}

/// The highest API level installed as either sources or a platform jar.
fn newest_installed_level(sdk_root: &Path) -> Option<u32> {
    let sources = highest_api_level(&sdk_root.join("sources"));
    let platforms = highest_api_level(&sdk_root.join("platforms"));
    sources.max(platforms)
}

/// The dep root serving one platform level, preferring the ready-made
/// `sources/android-<N>` tree `sdkmanager "sources;android-<N>"` installs
/// over the `.class` bytecode in `platforms/android-<N>/android.jar`.
fn platform_root(sdk_root: &Path, api_level: u32) -> Option<ExternalDepRoot> {
    let sources_dir = sdk_root
        .join("sources")
        .join(format!("android-{api_level}"));
    if sources_dir.is_dir() {
        debug!(
            "Android SDK sources API {api_level} registered at {}",
            sources_dir.display()
        );
        return Some(dep_root(api_level, sources_dir));
    }

    let jar_path = sdk_root
        .join("platforms")
        .join(format!("android-{api_level}"))
        .join("android.jar");
    if !jar_path.is_file() {
        warn!(
            "android-sdk: project compiles against android-{api_level} but the install at {} \
             holds neither its sources nor its platform jar",
            sdk_root.display()
        );
        return None;
    }

    let cache_dir = sdk_root
        .join("bearwisdom-android-cache")
        .join(format!("android-{api_level}"));
    if !cache_dir.exists() || is_cache_stale(&jar_path, &cache_dir) {
        if let Err(e) = extract_java_sources_jar(&jar_path, &cache_dir) {
            debug!("Failed to extract android.jar: {e}");
            return None;
        }
    }

    debug!(
        "Android SDK API {api_level} registered at {}",
        cache_dir.display()
    );
    Some(dep_root(api_level, cache_dir))
}

fn dep_root(api_level: u32, root: PathBuf) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: format!("android-sdk:{api_level}"),
        version: api_level.to_string(),
        root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

/// The SDK install to read platforms from. `BEARWISDOM_ANDROID_SDK` is the
/// explicit override; the rest are the variables the Android toolchain
/// itself honours.
fn android_sdk_home() -> Option<PathBuf> {
    for var in ["BEARWISDOM_ANDROID_SDK", "ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        let Some(val) = std::env::var_os(var) else {
            continue;
        };
        let p = PathBuf::from(val);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// The highest `android-<N>` level directory under `dir`.
fn highest_api_level(dir: &Path) -> Option<u32> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_str()?;
            let n: u32 = s.strip_prefix("android-")?.parse().ok()?;
            if e.path().is_dir() {
                Some(n)
            } else {
                None
            }
        })
        .max()
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<AndroidSdkEcosystem>> = OnceLock::new();
    LOCATOR
        .get_or_init(|| Arc::new(AndroidSdkEcosystem))
        .clone()
}

#[cfg(test)]
#[path = "android_sdk_tests.rs"]
mod tests;
