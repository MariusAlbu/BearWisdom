// =============================================================================
// kotlin/externals.rs — now a stub
//
// Maven dependency discovery (formerly `JavaExternalsLocator` delegation) and
// Android SDK platform-jar probing have moved to `ecosystem::maven::MavenEcosystem`.
// The Android SDK probe will be promoted to its own `AndroidSdkEcosystem` in
// Phase 5 of the refactor plan.
// =============================================================================

/// JVM runtime symbols are indexed from Maven source jars + Android SDK via
/// the Maven ecosystem. No hardcoded externals needed here.
pub(crate) const EXTERNALS: &[&str] = &[];
