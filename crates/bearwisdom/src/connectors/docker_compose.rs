// =============================================================================
// connectors/docker_compose.rs — Docker Compose connector
//
// Standalone post-index hook (not a Connector trait impl).
//
// For each compose file found in the project:
//   1. Parse services with a `build` context.
//   2. Map each build context to a package by matching the package path.
//   3. For each `depends_on` entry, write a flow_edge from the depender's
//      package to the dependency's package.
//
// Services backed only by external images (e.g. `postgres:15`) cannot be
// mapped to a package and are skipped — no code, no edge.
//
// edge_type: "service_dependency"
// protocol:  "infrastructure"
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_yaml::Value;
use tracing::warn;

use super::types::Protocol;
use crate::db::Database;

/// Compose file names searched in the project root and common sub-directories.
const COMPOSE_NAMES: &[&str] = &[
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
    // Override/local files often have build contexts that the production
    // compose files replace with pre-built images.
    "docker-compose.override.yml",
    "docker-compose.override.yaml",
    "compose.override.yml",
    "compose.override.yaml",
    "compose.local.yml",
    "compose.local.yaml",
    "docker-compose.local.yml",
    "docker-compose.local.yaml",
];

/// Sub-directories to check for compose files in addition to the project root.
const COMPOSE_SUBDIRS: &[&str] = &["deploy", "docker", "infra", "infrastructure", ".docker"];

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Metadata extracted for a single compose service.
struct ServiceInfo {
    /// The resolved package_id (None for external/image-only services).
    package_id: Option<i64>,
    /// Logical names of services this one depends on.
    depends_on: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Connect Docker Compose service dependencies into the graph.
///
/// For each compose file found:
/// 1. Parse services with `build` contexts.
/// 2. Map build contexts to packages (via packages table paths).
/// 3. For each `depends_on`, write a flow_edge from the depender's package to
///    the dependency's package.
///
/// Returns the number of flow_edges written.
pub fn connect(db: &Database, project_root: &Path) -> Result<u32> {
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
                // External image — no code to link.
                continue;
            };
            let depender_file_id = match representative_file_id(conn, depender_pkg_id) {
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
                    None => {
                        // External service (e.g. postgres image) — skip.
                        continue;
                    }
                };

                let dep_file_id = match representative_file_id(conn, dep_pkg_id) {
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

// ---------------------------------------------------------------------------
// Package resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a compose build context string to a package_id.
///
/// `build_context` is relative to the compose file's directory.
fn resolve_package_id(
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

    // Exact path match (try both with and without leading "./").
    conn.query_row(
        "SELECT id FROM packages WHERE path = ?1 OR path = ?2 LIMIT 1",
        rusqlite::params![normalised, format!("./{normalised}")],
        |row| row.get(0),
    )
    .ok()
    .or_else(|| {
        // Suffix/prefix match for edge cases (e.g. "." in a subdirectory context).
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

/// Get a representative file_id for a package (any source file in the package).
fn representative_file_id(conn: &rusqlite::Connection, package_id: i64) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM files WHERE package_id = ?1 LIMIT 1",
        rusqlite::params![package_id],
        |row| row.get(0),
    )
    .ok()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Return absolute paths to all compose files found in the project.
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

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Parse a Compose document and return a map of service_name → ServiceInfo,
/// with package_ids resolved from the database.
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

        // Resolve build context → package_id.  Services with only an `image`
        // key (no `build`) get None — they are external and can't be linked.
        let package_id = svc_val.get("build").and_then(|b| {
            let ctx = b
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| b.get("context").and_then(|c| c.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| ".".to_string());

            // Try build context first.
            resolve_package_id(conn, project_root, compose_dir, &ctx)
                .or_else(|| {
                    // If build context is "." (root), try the Dockerfile's directory
                    // as the package path.  e.g. `dockerfile: apps/api/Dockerfile`
                    // implies the service belongs to the `apps/api` package.
                    b.get("dockerfile")
                        .and_then(|d| d.as_str())
                        .and_then(|df| {
                            let df_dir = Path::new(df).parent()?;
                            if df_dir.as_os_str().is_empty() {
                                return None;
                            }
                            let df_rel = df_dir.to_string_lossy().replace('\\', "/");
                            resolve_package_id(conn, project_root, compose_dir, &df_rel)
                        })
                })
        });

        let depends_on = collect_depends_on(svc_val);

        services.insert(service_name, ServiceInfo { package_id, depends_on });
    }

    services
}

/// Collect port strings from a service value.
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

/// Collect service names from `depends_on` (both list and map forms).
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

    // ---------------------------------------------------------------------------
    // YAML parsing helpers
    // ---------------------------------------------------------------------------

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

    // ---------------------------------------------------------------------------
    // Package mapping
    // ---------------------------------------------------------------------------

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
    fn test_extract_depends_on_long_form_service() {
        let yaml = r#"
services:
  worker:
    build: .
    depends_on:
      redis:
        condition: service_healthy
      db:
        condition: service_started
  redis:
    image: redis:7
  db:
    image: postgres:15
"#;
        let doc = make_doc(yaml);
        let conn = in_memory_db_with_packages();
        let root = Path::new("/project");
        let compose = root.join("docker-compose.yml");
        let services = extract_services_with_packages(&conn, root, &compose, &doc);

        let worker_deps = &services["worker"].depends_on;
        assert_eq!(worker_deps.len(), 2);
        assert!(worker_deps.contains(&"redis".to_string()));
        assert!(worker_deps.contains(&"db".to_string()));
    }

    // ---------------------------------------------------------------------------
    // Discovery
    // ---------------------------------------------------------------------------

    #[test]
    fn test_find_compose_files_nonexistent_root() {
        let root = Path::new("/nonexistent_bearwisdom_test_root_xyz_abc");
        assert!(find_compose_files(root).is_empty());
    }
}
