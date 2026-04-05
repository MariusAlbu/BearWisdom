// indexer/manifest/go_mod.rs — go.mod reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct GoModManifest;

impl ManifestReader for GoModManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::GoMod
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let go_mod_path = find_go_mod(project_root)?;
        let content = std::fs::read_to_string(&go_mod_path).ok()?;

        let parsed = parse_go_mod(&content);
        let mut data = ManifestData::default();

        data.module_path = parsed.module_path;
        for path in parsed.require_paths {
            data.dependencies.insert(path);
        }

        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Parsed data from a go.mod file.
pub struct GoModData {
    /// The `module` directive value (e.g., "code.gitea.io/gitea").
    pub module_path: Option<String>,
    /// All module paths from `require` blocks (e.g., "github.com/gin-gonic/gin").
    pub require_paths: Vec<String>,
}

/// Find go.mod, checking the project root first, then immediate subdirectories
/// (depth 1) to handle monorepos where Go lives in e.g. `server/` or `backend/`.
pub fn find_go_mod(root: &Path) -> Option<std::path::PathBuf> {
    let candidate = root.join("go.mod");
    if candidate.is_file() {
        return Some(candidate);
    }
    // Check one level of subdirectories for monorepo layouts.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let nested = entry.path().join("go.mod");
                if nested.is_file() {
                    return Some(nested);
                }
            }
        }
    }
    None
}

/// Parse the `module` directive and `require` blocks from go.mod content.
///
/// go.mod format:
/// ```text
/// module code.gitea.io/gitea
///
/// go 1.21
///
/// require (
///     github.com/gin-gonic/gin v1.9.1
///     golang.org/x/crypto v0.14.0
/// )
///
/// require github.com/some/pkg v1.0.0
/// ```
pub fn parse_go_mod(content: &str) -> GoModData {
    let mut module_path: Option<String> = None;
    let mut require_paths = Vec::new();
    let mut in_require_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        // `module <path>`
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let path = rest.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() {
                module_path = Some(path.to_string());
            }
            continue;
        }

        // `require (` — start of multi-line block.
        if trimmed == "require (" || trimmed.starts_with("require (") {
            in_require_block = true;
            continue;
        }

        // `)` — end of a block.
        if trimmed == ")" {
            in_require_block = false;
            continue;
        }

        // Single-line `require <path> <version>`.
        if let Some(rest) = trimmed.strip_prefix("require ") {
            let path = rest.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() && path != "(" {
                require_paths.push(path.to_string());
            }
            continue;
        }

        // Inside a require block: `<path> <version>` or `<path> <version> // indirect`.
        if in_require_block {
            let path = trimmed.split_whitespace().next().unwrap_or("").trim();
            if !path.is_empty() && !path.starts_with("//") {
                require_paths.push(path.to_string());
            }
        }
    }

    GoModData { module_path, require_paths }
}
