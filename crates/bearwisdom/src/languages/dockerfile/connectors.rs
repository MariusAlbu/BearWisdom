// =============================================================================
// languages/dockerfile/connectors.rs — Dockerfile language plugin connectors
//
// Contains the Docker Compose post-index hook (inlined from
// connectors/docker_compose.rs). Docker Compose files are not Dockerfile
// language files, but they belong conceptually to the same infrastructure
// layer, so this plugin owns the hook.
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_yaml::Value;
use tracing::warn;

use crate::connectors::types::Protocol;
use crate::db::Database;

/// Detect Docker Compose service dependencies and write flow_edges.
///
/// Called from `DockerfilePlugin::post_index()`.
pub fn run_docker_compose(db: &Database, project_root: &Path) {
    match docker_compose_connect(db, project_root) {
        Ok(n) if n > 0 => tracing::info!(n, "Docker Compose service dependency edges"),
        Err(e) => warn!("Docker Compose connector: {e}"),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Docker Compose connector (inlined from connectors/docker_compose.rs)
// ---------------------------------------------------------------------------

const COMPOSE_NAMES: &[&str] = &[
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
    "docker-compose.override.yml",
    "docker-compose.override.yaml",
    "compose.override.yml",
    "compose.override.yaml",
    "compose.local.yml",
    "compose.local.yaml",
    "docker-compose.local.yml",
    "docker-compose.local.yaml",
];

const COMPOSE_SUBDIRS: &[&str] = &["deploy", "docker", "infra", "infrastructure", ".docker"];

struct ServiceInfo {
    package_id: Option<i64>,
    depends_on: Vec<String>,
}

fn docker_compose_connect(db: &Database, project_root: &Path) -> Result<u32> {
    let compose_files = find_compose_files(project_root);
    if compose_files.is_empty() {
        return Ok(0);
    }

    let conn = db.conn();
    let mut total = 0u32;
    tracing::debug!("docker_compose: found {} compose files", compose_files.len());

    for compose_path in compose_files {
        let content = match std::fs::read_to_string(&compose_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("docker_compose: cannot read {}: {e}", compose_path.display());
                continue;
            }
        };

        let doc: Value = match serde_yaml::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                warn!("docker_compose: failed to parse {}: {e}", compose_path.display());
                continue;
            }
        };

        let services = extract_services_with_packages(conn, project_root, &compose_path, &doc);
        tracing::debug!(
            "docker_compose: {} — {} services ({} with packages)",
            compose_path.display(),
            services.len(),
            services.values().filter(|s| s.package_id.is_some()).count(),
        );

        for (depender_name, service) in &services {
            let Some(depender_pkg_id) = service.package_id else {
                continue;
            };
            let depender_file_id = match dc_representative_file_id(conn, depender_pkg_id) {
                Some(id) => id,
                None => {
                    warn!(
                        "docker_compose: no files for package_id={depender_pkg_id} \
                         (service '{depender_name}') — skipping"
                    );
                    continue;
                }
            };

            for dep_name in &service.depends_on {
                let dep_pkg_id = match services.get(dep_name).and_then(|s| s.package_id) {
                    Some(id) => id,
                    None => continue,
                };

                let dep_file_id = match dc_representative_file_id(conn, dep_pkg_id) {
                    Some(id) => id,
                    None => {
                        warn!(
                            "docker_compose: no files for package_id={dep_pkg_id} \
                             (service '{dep_name}') — skipping"
                        );
                        continue;
                    }
                };

                let metadata = serde_json::json!({
                    "depender": depender_name,
                    "dependency": dep_name,
                })
                .to_string();

                match conn.prepare_cached(
                    "INSERT OR IGNORE INTO flow_edges
                        (source_file_id, source_line, source_symbol, source_language,
                         target_file_id, target_line, target_symbol, target_language,
                         edge_type, protocol, confidence, metadata)
                     VALUES (?1, 1, ?2, NULL, ?3, 1, ?4, NULL,
                             'service_dependency', ?5, 0.9, ?6)",
                ) {
                    Ok(mut stmt) => match stmt.execute(rusqlite::params![
                        depender_file_id,
                        depender_name,
                        dep_file_id,
                        dep_name,
                        Protocol::Infrastructure.as_str(),
                        metadata,
                    ]) {
                        Ok(n) => total += n as u32,
                        Err(e) => warn!(
                            "docker_compose: failed to insert flow_edge \
                             '{depender_name}' → '{dep_name}': {e}"
                        ),
                    },
                    Err(e) => warn!("docker_compose: prepare_cached failed: {e}"),
                }
            }
        }
    }

    Ok(total)
}

fn dc_resolve_package_id(
    conn: &rusqlite::Connection,
    project_root: &Path,
    compose_dir: &Path,
    build_context: &str,
) -> Option<i64> {
    let abs = if Path::new(build_context).is_absolute() {
        PathBuf::from(build_context)
    } else {
        compose_dir.join(build_context)
    };

    let rel = abs
        .strip_prefix(project_root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| build_context.replace('\\', "/"));

    let normalised = rel
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();

    conn.query_row(
        "SELECT id FROM packages WHERE path = ?1 OR path = ?2 LIMIT 1",
        rusqlite::params![normalised, format!("./{normalised}")],
        |row| row.get(0),
    )
    .ok()
    .or_else(|| {
        if normalised.is_empty() || normalised == "." {
            return None;
        }
        conn.query_row(
            "SELECT id FROM packages WHERE path LIKE ('%' || ?1) LIMIT 1",
            rusqlite::params![normalised],
            |row| row.get(0),
        )
        .ok()
    })
}

fn dc_representative_file_id(conn: &rusqlite::Connection, package_id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM files WHERE package_id = ?1 LIMIT 1",
        rusqlite::params![package_id],
        |row| row.get(0),
    )
    .ok()
}

fn find_compose_files(project_root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for name in COMPOSE_NAMES {
        let candidate = project_root.join(name);
        if candidate.is_file() {
            found.push(candidate);
        }
    }
    for subdir in COMPOSE_SUBDIRS {
        let dir = project_root.join(subdir);
        if dir.is_dir() {
            for name in COMPOSE_NAMES {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    found.push(candidate);
                }
            }
        }
    }
    found
}

fn extract_services_with_packages(
    conn: &rusqlite::Connection,
    project_root: &Path,
    compose_path: &Path,
    doc: &Value,
) -> HashMap<String, ServiceInfo> {
    let compose_dir = compose_path.parent().unwrap_or(project_root);
    let mut services = HashMap::new();

    let svc_map = match doc.get("services").and_then(|v| v.as_mapping()) {
        Some(m) => m,
        None => return services,
    };

    for (svc_key, svc_val) in svc_map {
        let service_name = match svc_key.as_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        let package_id = svc_val.get("build").and_then(|b| {
            let ctx = b
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| b.get("context").and_then(|c| c.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| ".".to_string());

            dc_resolve_package_id(conn, project_root, compose_dir, &ctx)
                .or_else(|| {
                    b.get("dockerfile")
                        .and_then(|d| d.as_str())
                        .and_then(|df| {
                            let df_dir = Path::new(df).parent()?;
                            if df_dir.as_os_str().is_empty() {
                                return None;
                            }
                            let df_rel = df_dir.to_string_lossy().replace('\\', "/");
                            dc_resolve_package_id(conn, project_root, compose_dir, &df_rel)
                        })
                })
        });

        let depends_on = collect_depends_on(svc_val);
        services.insert(service_name, ServiceInfo { package_id, depends_on });
    }

    services
}

fn collect_depends_on(svc: &Value) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(dep_val) = svc.get("depends_on") else {
        return deps;
    };
    if let Some(list) = dep_val.as_sequence() {
        for item in list {
            if let Some(name) = item.as_str() {
                deps.push(name.to_string());
            }
        }
        return deps;
    }
    if let Some(map) = dep_val.as_mapping() {
        for (key, _) in map {
            if let Some(name) = key.as_str() {
                deps.push(name.to_string());
            }
        }
    }
    deps
}

fn collect_ports(svc: &Value) -> Vec<String> {
    let mut ports = Vec::new();
    if let Some(port_list) = svc.get("ports").and_then(|v| v.as_sequence()) {
        for p in port_list {
            if let Some(s) = p.as_str() {
                ports.push(s.to_string());
            } else if let Some(n) = p.as_u64() {
                ports.push(n.to_string());
            } else if let Some(m) = p.as_mapping() {
                if let Some(target) = m.get("target").and_then(|v| v.as_u64()) {
                    if let Some(published) = m.get("published").and_then(|v| v.as_u64()) {
                        ports.push(format!("{published}:{target}"));
                    } else {
                        ports.push(target.to_string());
                    }
                }
            }
        }
    }
    ports
}

// ---------------------------------------------------------------------------
// Dockerfile detection (inlined from connectors/dockerfile.rs)
// ---------------------------------------------------------------------------
//
// NOT a Connector implementation. Standalone post-index function called from
// full.rs after package detection to mark packages with a Dockerfile as
// deployable services (`is_service = 1`).

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
pub fn detect_dockerfiles(conn: &rusqlite::Connection, project_root: &Path) -> Vec<(String, String)> {
    let packages = match load_package_paths(conn) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("dockerfile: failed to load packages: {e}");
            return Vec::new();
        }
    };

    let dockerfiles = scan_dockerfiles(project_root);
    if dockerfiles.is_empty() {
        return Vec::new();
    }

    let mut pairs = Vec::new();

    for dockerfile_rel in &dockerfiles {
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
            tracing::debug!("dockerfile: {} → package {}", dockerfile_rel, pkg_path);
            pairs.push((pkg_path.clone(), dockerfile_rel.clone()));
        } else if !packages.is_empty() {
            tracing::debug!("dockerfile: {} — no matching package", dockerfile_rel);
        }
    }

    pairs
}

fn scan_dockerfiles(project_root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    scan_dir_for_dockerfiles(project_root, project_root, &mut found, 0);
    found
}

fn scan_dir_for_dockerfiles(root: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 5 { return; }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if should_skip_dir(&name) { continue; }
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
    if DOCKERFILE_NAMES.contains(&name) { return true; }
    for prefix in DOCKERFILE_PREFIXES {
        if name.starts_with(prefix) { return true; }
    }
    for suffix in DOCKERFILE_SUFFIXES {
        if name.ends_with(suffix) { return true; }
    }
    false
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | "target" | ".git" | ".svn" | "vendor"
            | "__pycache__" | ".venv" | "venv" | "dist" | "build"
            | ".idea" | ".vscode"
    )
}

fn load_package_paths(conn: &rusqlite::Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT path FROM packages")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut paths = Vec::new();
    for row in rows { paths.push(row?); }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_doc(yaml: &str) -> Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn in_memory_db_with_packages() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE packages (
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL,
                 name TEXT,
                 kind TEXT,
                 is_service INTEGER DEFAULT 0
             );
             CREATE TABLE files (
                 id INTEGER PRIMARY KEY,
                 path TEXT NOT NULL,
                 package_id INTEGER
             );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_collect_depends_on_short_form() {
        let yaml = "depends_on:\n  - db\n  - redis\n";
        let doc = make_doc(yaml);
        let deps = collect_depends_on(&doc);
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"db".to_string()));
        assert!(deps.contains(&"redis".to_string()));
    }

    #[test]
    fn test_collect_depends_on_long_form() {
        let yaml = r#"
depends_on:
  redis:
    condition: service_healthy
  db:
    condition: service_started
"#;
        let doc = make_doc(yaml);
        let deps = collect_depends_on(&doc);
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"redis".to_string()));
        assert!(deps.contains(&"db".to_string()));
    }

    #[test]
    fn test_collect_depends_on_absent() {
        let yaml = "image: postgres:15\n";
        let doc = make_doc(yaml);
        assert!(collect_depends_on(&doc).is_empty());
    }

    #[test]
    fn test_collect_ports_long_form() {
        let yaml = "ports:\n  - target: 80\n    published: 8080\n";
        let doc = make_doc(yaml);
        assert_eq!(collect_ports(&doc), vec!["8080:80"]);
    }

    #[test]
    fn test_collect_ports_short_form() {
        let yaml = "ports:\n  - \"8080:8080\"\n  - \"5432\"\n";
        let doc = make_doc(yaml);
        assert_eq!(collect_ports(&doc), vec!["8080:8080", "5432"]);
    }

    #[test]
    fn test_no_services_key_returns_empty() {
        let yaml = "version: '3.8'\nname: test\n";
        let doc = make_doc(yaml);
        let conn = in_memory_db_with_packages();
        let root = Path::new("/project");
        let compose = root.join("docker-compose.yml");
        let result = extract_services_with_packages(&conn, root, &compose, &doc);
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_service_with_build_context_maps_package() {
        let yaml = r#"
services:
  api:
    build: ./api
    ports:
      - "8080:8080"
    depends_on:
      - db
  db:
    build:
      context: ./db
    image: postgres:15
  cache:
    image: redis:7
"#;
        let doc = make_doc(yaml);
        let conn = in_memory_db_with_packages();
        conn.execute_batch(
            "INSERT INTO packages (id, path, name) VALUES (1, 'api', 'api');
             INSERT INTO packages (id, path, name) VALUES (2, 'db', 'db');",
        )
        .unwrap();

        let root = Path::new("/project");
        let compose = root.join("docker-compose.yml");
        let services = extract_services_with_packages(&conn, root, &compose, &doc);

        assert!(services.contains_key("api"));
        assert!(services.contains_key("db"));
        assert!(services.contains_key("cache"));

        assert_eq!(services["api"].package_id, Some(1));
        assert_eq!(services["db"].package_id, Some(2));
        assert_eq!(services["cache"].package_id, None, "image-only service has no package");
        assert!(services["api"].depends_on.contains(&"db".to_string()));
    }

    #[test]
    fn test_find_compose_files_nonexistent_root() {
        let root = Path::new("/nonexistent_bearwisdom_test_root_xyz_abc");
        assert!(find_compose_files(root).is_empty());
    }

    // Dockerfile detection tests (from connectors/dockerfile.rs)

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
