// =============================================================================
// connectors/dockerfile.rs — Dockerfile detection
//
// This is NOT a Connector implementation.  It is a standalone post-index
// function called from full.rs after package detection to mark packages that
// have a Dockerfile as deployable services (`is_service = 1`).
//
// Returns: (package_relative_path, dockerfile_relative_path) pairs.
// =============================================================================

use std::path::Path;

use rusqlite::Connection;
use tracing::{debug, warn};

/// Dockerfile filename patterns to scan for.
const DOCKERFILE_NAMES: &[&str] = &["Dockerfile"];
const DOCKERFILE_PREFIXES: &[&str] = &["Dockerfile."];
const DOCKERFILE_SUFFIXES: &[&str] = &[".dockerfile", ".Dockerfile"];

/// Detect Dockerfiles in `project_root` and return `(package_path, dockerfile_path)` pairs
/// by matching each Dockerfile against the nearest package in the `packages` table.
///
/// Both paths are relative to the project root (as stored in the DB).
///
/// Called from `full.rs` after packages are written; the result is used to set
/// `is_service = 1` on matching packages.
pub fn detect_dockerfiles(conn: &Connection, project_root: &Path) -> Vec<(String, String)> {
    // Load all package paths from DB so we can do path-prefix matching.
    let packages = match load_package_paths(conn) {
        Ok(p) => p,
        Err(e) => {
            warn!("dockerfile: failed to load packages: {e}");
            return Vec::new();
        }
    };

    // Scan for Dockerfiles.
    let dockerfiles = scan_dockerfiles(project_root);
    if dockerfiles.is_empty() {
        return Vec::new();
    }

    let mut pairs = Vec::new();

    for dockerfile_rel in &dockerfiles {
        // Find the deepest package whose path is a prefix of the Dockerfile's path.
        let best = packages
            .iter()
            .filter(|pkg_path| {
                let dockerfile_normalized = dockerfile_rel.replace('\\', "/");
                let pkg_normalized = pkg_path.replace('\\', "/");
                dockerfile_normalized == pkg_normalized
                    || dockerfile_normalized.starts_with(&format!("{pkg_normalized}/"))
            })
            .max_by_key(|pkg_path| pkg_path.len());

        if let Some(pkg_path) = best {
            debug!(
                "dockerfile: {} → package {}",
                dockerfile_rel, pkg_path
            );
            pairs.push((pkg_path.clone(), dockerfile_rel.clone()));
        } else if !packages.is_empty() {
            // If there are packages but none matched, the Dockerfile is at the
            // workspace root — skip; it's not unambiguously owned by any package.
            debug!("dockerfile: {} — no matching package", dockerfile_rel);
        }
        // If there are no packages at all (single-project repo), return a sentinel
        // so the caller can set is_service on the implicit root "package".
    }

    pairs
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

fn scan_dockerfiles(project_root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    scan_dir_for_dockerfiles(project_root, project_root, &mut found, 0);
    found
}

fn scan_dir_for_dockerfiles(
    root: &Path,
    dir: &Path,
    out: &mut Vec<String>,
    depth: usize,
) {
    // Cap recursion — Dockerfiles are almost never buried more than 4 levels deep.
    if depth > 5 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        // Skip common non-source directories.
        if path.is_dir() {
            if should_skip_dir(&name) {
                continue;
            }
            scan_dir_for_dockerfiles(root, &path, out, depth + 1);
            continue;
        }

        if is_dockerfile_name(&name) {
            if let Ok(rel) = path.strip_prefix(root) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                out.push(rel_str);
            }
        }
    }
}

fn is_dockerfile_name(name: &str) -> bool {
    if DOCKERFILE_NAMES.contains(&name) {
        return true;
    }
    for prefix in DOCKERFILE_PREFIXES {
        if name.starts_with(prefix) {
            return true;
        }
    }
    for suffix in DOCKERFILE_SUFFIXES {
        if name.ends_with(suffix) {
            return true;
        }
    }
    false
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules"
            | "target"
            | ".git"
            | ".svn"
            | "vendor"
            | "__pycache__"
            | ".venv"
            | "venv"
            | "dist"
            | "build"
            | ".idea"
            | ".vscode"
    )
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

fn load_package_paths(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT path FROM packages")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut paths = Vec::new();
    for row in rows {
        paths.push(row?);
    }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dockerfile_name() {
        assert!(is_dockerfile_name("Dockerfile"));
        assert!(is_dockerfile_name("Dockerfile.prod"));
        assert!(is_dockerfile_name("Dockerfile.dev"));
        assert!(is_dockerfile_name("app.dockerfile"));
        assert!(is_dockerfile_name("app.Dockerfile"));
        assert!(!is_dockerfile_name("docker-compose.yml"));
        assert!(!is_dockerfile_name("README.md"));
        assert!(!is_dockerfile_name("dockerfile")); // lowercase — not matched by convention
    }

    #[test]
    fn test_should_skip_dir() {
        assert!(should_skip_dir("node_modules"));
        assert!(should_skip_dir("target"));
        assert!(should_skip_dir(".git"));
        assert!(!should_skip_dir("src"));
        assert!(!should_skip_dir("services"));
    }
}
