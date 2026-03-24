use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// ScannedFile — produced by walker, consumed by the index crate.
// ---------------------------------------------------------------------------

/// A source file discovered during the project walk.
///
/// Produced by [`walker::walk_files`] and consumed by the index crate.
/// Never crosses IPC — stays in Rust.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    /// Path relative to project root, forward-slash normalized.
    pub relative_path: String,
    /// Absolute path on disk — used for reading file contents.
    pub absolute_path: PathBuf,
    /// Language identifier from the profile registry (e.g. "typescript", "rust").
    pub language_id: &'static str,
}

// ---------------------------------------------------------------------------
// Shell-platform commands — all &'static because they live in static descriptors.
// Serialize only: these are write-once static data, never deserialized.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellCommands {
    pub bash: &'static str,
    pub powershell: &'static str,
    pub cmd: &'static str,
}

impl ShellCommands {
    /// Convenience: same command string for all shells.
    pub const fn same(cmd: &'static str) -> Self {
        Self { bash: cmd, powershell: cmd, cmd }
    }
}

// ---------------------------------------------------------------------------
// SdkDescriptor
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SdkDescriptor {
    /// Human-readable SDK name, e.g. "Rust (rustc)".
    pub name: &'static str,
    /// Executable to run for version check, e.g. "rustc".
    pub version_command: &'static str,
    /// Args to pass, e.g. &["--version"].
    pub version_args: &'static [&'static str],
    /// Optional file that pins the version, e.g. "rust-toolchain.toml".
    pub version_file: Option<&'static str>,
    /// If the version file is JSON, the key holding the version string.
    pub version_json_key: Option<&'static str>,
    /// URL for human install instructions.
    pub install_url: &'static str,
}

// ---------------------------------------------------------------------------
// PmDescriptor — package manager
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PmDescriptor {
    /// e.g. "cargo", "npm", "pip"
    pub name: &'static str,
    /// Lock file that indicates this PM is in use, e.g. "Cargo.lock".
    pub lock_file: Option<&'static str>,
    /// Local deps/vendor directory, e.g. "node_modules".
    pub deps_dir: Option<&'static str>,
    /// Full install command (from scratch).
    pub install_cmd: ShellCommands,
    /// Restore/sync command (lock file present, re-download).
    pub restore_cmd: ShellCommands,
}

// ---------------------------------------------------------------------------
// TfDescriptor — test framework
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TfDescriptor {
    /// e.g. "vitest"
    pub name: &'static str,
    /// e.g. "Vitest"
    pub display_name: &'static str,
    /// Config/marker files that indicate this framework is present.
    pub config_files: &'static [&'static str],
    /// Optional substring to match inside a config file.
    pub config_content_match: Option<&'static str>,
    /// Optional package.json dep name for JS frameworks.
    pub package_json_dep: Option<&'static str>,
    /// Command to discover all tests.
    pub discovery_cmd: Option<ShellCommands>,
    /// Command to run all tests.
    pub run_cmd: ShellCommands,
    /// Command to run a single test file (use "{file}" placeholder).
    pub run_single_cmd: ShellCommands,
}

// ---------------------------------------------------------------------------
// RestoreTrigger / RestoreStep
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RestoreTrigger {
    /// A directory is absent (e.g. node_modules missing).
    DirMissing,
    /// A required file is absent (e.g. .env missing).
    FileMissing,
    /// A file exists but should be transformed (e.g. .env.example present).
    FileExists,
    /// Installed SDK version doesn't match the pinned version file.
    SdkVersionMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreStep {
    /// Unique id, e.g. "cargo-fetch".
    pub id: &'static str,
    /// Short human title.
    pub title: &'static str,
    /// Longer description shown to the user.
    pub description: &'static str,
    /// When to trigger this step.
    pub trigger: RestoreTrigger,
    /// The path/name being watched (dir name, file name, etc.).
    pub watch_path: &'static str,
    /// Commands to execute the restoration.
    pub commands: ShellCommands,
    /// Whether the app can run this automatically (no user confirmation needed).
    pub auto_fixable: bool,
    /// If true, the project will likely not build/run without this step.
    pub critical: bool,
}

// ---------------------------------------------------------------------------
// LanguageDescriptor — the atomic unit. Serialize only.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageDescriptor {
    /// Stable machine id, e.g. "rust".
    pub id: &'static str,
    /// Display name, e.g. "Rust".
    pub display_name: &'static str,
    /// File extensions (with leading dot), e.g. &[".rs"].
    pub file_extensions: &'static [&'static str],
    /// Exact filenames (no extension), e.g. &["Dockerfile", "Makefile"].
    pub filenames: &'static [&'static str],
    /// Alternative ids, e.g. &["rs"] for Rust.
    pub aliases: &'static [&'static str],
    /// Build output / generated dirs to exclude from indexing.
    pub exclude_dirs: &'static [&'static str],
    /// Project manifest / entry-point filenames, e.g. &["Cargo.toml"].
    pub entry_point_files: &'static [&'static str],
    /// SDK version check config. None for markup/data languages.
    pub sdk: Option<SdkDescriptor>,
    /// Package managers used in this ecosystem.
    pub package_managers: &'static [PmDescriptor],
    /// Test frameworks for this language.
    pub test_frameworks: &'static [TfDescriptor],
    /// Restore steps needed to get from clean checkout to runnable.
    pub restore_steps: &'static [RestoreStep],
    /// Single-line comment prefix, e.g. "//".
    pub line_comment: Option<&'static str>,
    /// Block comment open/close, e.g. ("/*", "*/").
    pub block_comment: Option<(&'static str, &'static str)>,
}

// ---------------------------------------------------------------------------
// ProjectProfile — scan result. Full Serialize + Deserialize.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageStats {
    pub language_id: String,
    pub display_name: String,
    pub file_count: usize,
    /// Entry-point files found for this language (relative paths).
    pub entry_points: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedSdk {
    pub language_id: String,
    pub sdk_name: String,
    /// Installed version string, or None if the SDK was not found.
    pub installed_version: Option<String>,
    /// Version pinned by a project file (e.g. global.json), or None.
    pub pinned_version: Option<String>,
    pub install_url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedPackageManager {
    pub language_id: String,
    pub name: String,
    /// Whether the lock file exists.
    pub has_lock_file: bool,
    /// Whether the deps directory exists.
    pub deps_installed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedTestFramework {
    pub language_id: String,
    pub name: String,
    pub display_name: String,
    pub run_cmd: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonorepoInfo {
    pub kind: String,
    /// Sub-package root paths (relative).
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInfo {
    /// .env.example present but .env missing.
    pub missing_env_file: bool,
    /// docker-compose.yml or docker-compose.yaml present.
    pub has_docker_compose: bool,
    /// .env files found (relative paths).
    pub env_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProfile {
    /// Absolute root path that was scanned.
    pub root: String,
    /// Detected languages, sorted by file_count desc.
    pub languages: Vec<LanguageStats>,
    /// SDK detection results (one per language with an SDK).
    pub sdks: Vec<DetectedSdk>,
    /// Package manager results.
    pub package_managers: Vec<DetectedPackageManager>,
    /// Detected test frameworks.
    pub test_frameworks: Vec<DetectedTestFramework>,
    /// Monorepo/workspace info if detected.
    pub monorepo: Option<MonorepoInfo>,
    /// Environment / docker compose info.
    pub environment: EnvironmentInfo,
    /// Pending restore steps (triggers that fired).
    pub restore_steps: Vec<String>,
    /// Extra metadata for consumers (extensible bag).
    pub meta: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// ScanOptions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Run SDK version commands. Set false to skip the slow I/O.
    pub check_sdks: bool,
    /// Max depth for entry-point file discovery.
    pub max_depth: usize,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self { check_sdks: true, max_depth: 3 }
    }
}
