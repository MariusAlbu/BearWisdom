//! # bearwisdom-profile
//!
//! Language-first project scanning. `LanguageDescriptor` is the atomic unit —
//! file detection, build exclusions, SDK checks, package managers, test
//! frameworks, and restore steps are all properties of the language ecosystem.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use bearwisdom_profile::{scan, ScanOptions};
//! use std::path::Path;
//!
//! let profile = scan(Path::new("/my/project"), ScanOptions::default());
//! println!("{} languages detected", profile.languages.len());
//! ```

pub mod detect;
pub mod exclusions;
pub mod languages;
pub mod registry;
pub mod scanner;
pub mod types;
pub mod walker;

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Flat public re-exports — callers should not need to know internal modules.
// ---------------------------------------------------------------------------

pub use detect::detect_language;
pub use exclusions::{
    build_walker, canonical_exclude_dirs, project_active_languages, project_exclude_dirs,
    should_exclude, should_exclude_in_project, should_exclude_in_project_path,
    COMMON_EXCLUDE_DIRS, ROOT_ONLY_EXCLUDE_NAMES,
};
pub use registry::{find_language, find_language_by_extension, LANGUAGES};
pub use scanner::{scan, scan_with_manifest, ScanResult};
pub use types::{
    DetectedPackageManager, DetectedSdk, DetectedTestFramework, EnvironmentInfo,
    LanguageDescriptor, LanguageStats, MonorepoInfo, PmDescriptor, ProjectProfile,
    RestoreStep, RestoreTrigger, ScanOptions, ScannedFile, SdkDescriptor, ShellCommands,
    TfDescriptor,
};
pub use walker::walk_files;
