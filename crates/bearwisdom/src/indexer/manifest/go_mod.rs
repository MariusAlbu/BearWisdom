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
    /// Full dependency records — path + version + indirect flag. Needed to
    /// resolve each require entry to its `$GOMODCACHE/{path}@{version}` dir
    /// when indexing external sources.
    pub require_deps: Vec<GoModDep>,
}

/// One `require` line, fully parsed.
#[derive(Debug, Clone)]
pub struct GoModDep {
    pub path: String,
    pub version: String,
    /// True if the line had a trailing `// indirect` marker.
    pub indirect: bool,
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
    let mut require_deps = Vec::new();
    let mut in_require_block = false;

    // Parse a `<path> <version> [// indirect]` fragment into a GoModDep.
    // Returns None if fewer than 2 tokens are present.
    fn parse_dep(fragment: &str) -> Option<GoModDep> {
        let without_comment = fragment.trim();
        let (main, comment) = match without_comment.find("//") {
            Some(idx) => (without_comment[..idx].trim(), &without_comment[idx..]),
            None => (without_comment, ""),
        };
        let mut tokens = main.split_whitespace();
        let path = tokens.next()?.to_string();
        let version = tokens.next()?.to_string();
        let indirect = comment.contains("indirect");
        Some(GoModDep { path, version, indirect })
    }

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
            let rest = rest.trim();
            if rest != "(" && !rest.is_empty() {
                if let Some(dep) = parse_dep(rest) {
                    require_paths.push(dep.path.clone());
                    require_deps.push(dep);
                }
            }
            continue;
        }

        // Inside a require block: `<path> <version>` or `<path> <version> // indirect`.
        if in_require_block && !trimmed.starts_with("//") {
            if let Some(dep) = parse_dep(trimmed) {
                require_paths.push(dep.path.clone());
                require_deps.push(dep);
            }
        }
    }

    GoModData { module_path, require_paths, require_deps }
}
