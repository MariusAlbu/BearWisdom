// =============================================================================
// ecosystem/android_sdk.rs — Android platform SDK (stdlib for Kotlin + Java)
//
// Extracted from ecosystem/maven.rs as part of Phase 5. Discovers
// `$ANDROID_HOME/platforms/android-<N>/android.jar`, extracts its .java
// stubs into a bearwisdom-owned cache, and hands the cache dir to the
// main indexer. Activation is transitive on Maven being active so we
// don't pay the probe cost on non-Android projects.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
};
use crate::indexer::externals::{
    extract_java_sources_jar, is_cache_stale, ExternalDepRoot, ExternalSourceLocator,
};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("android-sdk");
const LEGACY_ECOSYSTEM_TAG: &str = "android-sdk";
const LANGUAGES: &[&str] = &["kotlin", "java"];

pub struct AndroidSdkEcosystem;

impl Ecosystem for AndroidSdkEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::All(&[
            EcosystemActivation::TransitiveOn(super::maven::ID),
            EcosystemActivation::Any(&[
                EcosystemActivation::LanguagePresent("kotlin"),
                EcosystemActivation::LanguagePresent("java"),
            ]),
        ])
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_android_sdk_roots()
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

impl ExternalSourceLocator for AndroidSdkEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_android_sdk_roots()
    }
    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        super::maven::walk_generic_jvm_root(dep)
    }
}

pub(crate) fn discover_android_sdk_roots() -> Vec<ExternalDepRoot> {
    let Some(sdk_root) = android_home() else { return Vec::new() };
    let platforms_dir = sdk_root.join("platforms");
    if !platforms_dir.is_dir() { return Vec::new() }

    let api_level = match highest_api_level(&platforms_dir) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let platform_dir = platforms_dir.join(format!("android-{api_level}"));
    let jar_path = platform_dir.join("android.jar");
    if !jar_path.is_file() { return Vec::new() }

    let cache_base = sdk_root.join("bearwisdom-android-cache");
    let cache_dir = cache_base.join(format!("android-{api_level}"));
    if !cache_dir.exists() || is_cache_stale(&jar_path, &cache_dir) {
        if let Err(e) = extract_java_sources_jar(&jar_path, &cache_dir) {
            debug!("Failed to extract android.jar: {e}");
            return Vec::new();
        }
    }

    debug!("Android SDK API {api_level} registered at {}", cache_dir.display());
    vec![ExternalDepRoot {
        module_path: format!("android-sdk:{api_level}"),
        version: api_level.to_string(),
        root: cache_dir,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
    }]
}

fn android_home() -> Option<PathBuf> {
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(val) = std::env::var(var) {
            let p = PathBuf::from(val);
            if p.is_dir() { return Some(p) }
        }
    }
    None
}

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

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<AndroidSdkEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(AndroidSdkEcosystem)).clone()
}
