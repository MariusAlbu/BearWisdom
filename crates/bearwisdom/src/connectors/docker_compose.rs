// =============================================================================
// connectors/docker_compose.rs — Docker Compose connector
//
// Maps Compose service definitions and inter-service dependencies to
// ConnectionPoints, then matches them into flow_edges.
//
// Stop:  service with a `build` context (the service itself is a deployable unit)
// Start: service's `depends_on` entries (this service depends on another)
//
// Matching key: service name.  custom_match emits "service_dependency" edges.
// =============================================================================

use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;
use serde_yaml::Value;
use tracing::warn;

use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol, ResolvedFlow};
use crate::indexer::project_context::ProjectContext;

/// Compose file names searched in the project root and common sub-directories.
const COMPOSE_NAMES: &[&str] = &[
    "docker-compose.yml",
    "docker-compose.yaml",
    "compose.yml",
    "compose.yaml",
    // Override files are not primary descriptors — skip them to avoid double-counting.
];

/// Sub-directories to check for compose files in addition to the project root.
const COMPOSE_SUBDIRS: &[&str] = &["deploy", "docker", "infra", "infrastructure", ".docker"];

pub struct DockerComposeConnector;

impl Connector for DockerComposeConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "docker_compose",
            protocols: &[Protocol::Infrastructure],
            languages: &["yaml"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        // detect() is cheap — always run; extract() short-circuits if no files found.
        true
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let compose_files = find_compose_files(project_root);
        if compose_files.is_empty() {
            return Ok(Vec::new());
        }

        let mut points = Vec::new();

        for compose_path in compose_files {
            let file_id = match lookup_file_id(conn, &compose_path) {
                Some(id) => id,
                None => {
                    // File exists on disk but is not in the index — this can happen
                    // if the walker is run with a language filter that excludes YAML.
                    // Skip: we cannot emit valid ConnectionPoints without a file_id.
                    warn!(
                        "docker_compose: {} not in files table — skipping",
                        compose_path.display()
                    );
                    continue;
                }
            };

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

            extract_from_compose(file_id, &doc, &mut points);
        }

        Ok(points)
    }

    fn custom_match(
        &self,
        _conn: &Connection,
        starts: &[ConnectionPoint],
        stops: &[ConnectionPoint],
    ) -> Result<Option<Vec<ResolvedFlow>>> {
        // Match depends_on entries (Start) to their service definition (Stop) by
        // exact service name key.
        let mut flows = Vec::new();

        for start in starts {
            if start.framework != "docker-compose" {
                continue;
            }
            for stop in stops {
                if stop.framework == "docker-compose" && stop.key == start.key {
                    flows.push(ResolvedFlow {
                        start: start.clone(),
                        stop: stop.clone(),
                        confidence: 0.9,
                        edge_type: "service_dependency".to_string(),
                    });
                    // One service definition per name — no need to keep searching.
                    break;
                }
            }
        }

        Ok(Some(flows))
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Return absolute paths to all compose files found in the project.
fn find_compose_files(project_root: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();

    // Project root first.
    for name in COMPOSE_NAMES {
        let candidate = project_root.join(name);
        if candidate.is_file() {
            found.push(candidate);
        }
    }

    // Common sub-directories.
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

/// Look up the `files` table id for an absolute path.
///
/// Tries both the canonical path string and a LIKE pattern that matches the
/// path with either forward- or back-slashes, since BearWisdom normalises
/// paths to forward-slashes but the OS may return backslashes on Windows.
fn lookup_file_id(conn: &Connection, abs_path: &Path) -> Option<i64> {
    let path_str = abs_path.to_string_lossy().replace('\\', "/");

    // Exact match on normalised path.
    let result: rusqlite::Result<i64> = conn.query_row(
        "SELECT id FROM files WHERE path = ?1 LIMIT 1",
        rusqlite::params![path_str],
        |row| row.get(0),
    );
    if let Ok(id) = result {
        return Some(id);
    }

    // Suffix match — the stored path is relative to the project root.
    let file_name = abs_path.file_name()?.to_string_lossy().into_owned();
    let result: rusqlite::Result<i64> = conn.query_row(
        "SELECT id FROM files WHERE path LIKE ?1 LIMIT 1",
        rusqlite::params![format!("%/{file_name}")],
        |row| row.get(0),
    );
    result.ok()
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Parse a Compose document and push ConnectionPoints into `out`.
fn extract_from_compose(file_id: i64, doc: &Value, out: &mut Vec<ConnectionPoint>) {
    // Compose v2/v3: top-level `services` mapping.
    let services = match doc.get("services").and_then(|v| v.as_mapping()) {
        Some(m) => m,
        None => return,
    };

    for (svc_key, svc_val) in services {
        let service_name = match svc_key.as_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Stop point: service that has a `build` key (it is a local buildable image).
        if svc_val.get("build").is_some() {
            let build_context = svc_val
                .get("build")
                .and_then(|b| {
                    // `build` can be a plain string or a mapping with `context`.
                    b.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| b.get("context").and_then(|c| c.as_str()).map(|s| s.to_string()))
                })
                .unwrap_or_else(|| ".".to_string());

            let ports = collect_ports(svc_val);
            let image = svc_val
                .get("image")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let metadata = serde_json::json!({
                "build_context": build_context,
                "ports": ports,
                "image": image,
            })
            .to_string();

            out.push(ConnectionPoint {
                file_id,
                symbol_id: None,
                line: 1, // YAML line tracking is not available without a full YAML parser with spans
                protocol: Protocol::Infrastructure,
                direction: FlowDirection::Stop,
                key: service_name.clone(),
                method: String::new(),
                framework: "docker-compose".to_string(),
                metadata: Some(metadata),
            });
        }

        // Start points: each entry in `depends_on` (this service depends on another).
        for dep_name in collect_depends_on(svc_val) {
            out.push(ConnectionPoint {
                file_id,
                symbol_id: None,
                line: 1,
                protocol: Protocol::Infrastructure,
                direction: FlowDirection::Start,
                key: dep_name,
                method: String::new(),
                framework: "docker-compose".to_string(),
                metadata: Some(
                    serde_json::json!({ "depender": service_name }).to_string(),
                ),
            });
        }
    }
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
                // Long-form port syntax: {target: 8080, published: 8080}
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

    // Short form: list of service names.
    if let Some(list) = dep_val.as_sequence() {
        for item in list {
            if let Some(name) = item.as_str() {
                deps.push(name.to_string());
            }
        }
        return deps;
    }

    // Long form: mapping of service_name → {condition: ...}
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

    fn make_compose_doc(yaml: &str) -> Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn test_extract_service_with_build_and_depends_on() {
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
"#;
        let doc = make_compose_doc(yaml);
        let mut points = Vec::new();
        extract_from_compose(42, &doc, &mut points);

        // Stops: api (has build), db (has build)
        let stops: Vec<_> = points.iter().filter(|p| p.direction == FlowDirection::Stop).collect();
        assert_eq!(stops.len(), 2, "expected 2 stop points");
        assert!(stops.iter().any(|p| p.key == "api"));
        assert!(stops.iter().any(|p| p.key == "db"));

        // Start: api depends_on db
        let starts: Vec<_> = points.iter().filter(|p| p.direction == FlowDirection::Start).collect();
        assert_eq!(starts.len(), 1, "expected 1 start point (depends_on)");
        assert_eq!(starts[0].key, "db");

        // All have correct protocol and framework
        for p in &points {
            assert_eq!(p.protocol, Protocol::Infrastructure);
            assert_eq!(p.framework, "docker-compose");
            assert_eq!(p.file_id, 42);
        }
    }

    #[test]
    fn test_extract_depends_on_long_form() {
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
        let doc = make_compose_doc(yaml);
        let mut points = Vec::new();
        extract_from_compose(1, &doc, &mut points);

        let starts: Vec<_> = points.iter().filter(|p| p.direction == FlowDirection::Start).collect();
        // worker depends_on redis and db
        assert_eq!(starts.len(), 2);
        let dep_keys: Vec<_> = starts.iter().map(|p| p.key.as_str()).collect();
        assert!(dep_keys.contains(&"redis"));
        assert!(dep_keys.contains(&"db"));
    }

    #[test]
    fn test_extract_no_services_key() {
        let yaml = "version: '3.8'\nname: test\n";
        let doc = make_compose_doc(yaml);
        let mut points = Vec::new();
        extract_from_compose(1, &doc, &mut points);
        assert!(points.is_empty());
    }

    #[test]
    fn test_collect_ports_long_form() {
        let yaml = r#"
ports:
  - target: 80
    published: 8080
"#;
        let doc: Value = serde_yaml::from_str(yaml).unwrap();
        let ports = collect_ports(&doc);
        assert_eq!(ports, vec!["8080:80"]);
    }

    #[test]
    fn test_custom_match_produces_service_dependency_edges() {
        let connector = DockerComposeConnector;

        let start = ConnectionPoint {
            file_id: 1,
            symbol_id: None,
            line: 1,
            protocol: Protocol::Infrastructure,
            direction: FlowDirection::Start,
            key: "db".to_string(),
            method: String::new(),
            framework: "docker-compose".to_string(),
            metadata: None,
        };
        let stop = ConnectionPoint {
            file_id: 1,
            symbol_id: None,
            line: 1,
            protocol: Protocol::Infrastructure,
            direction: FlowDirection::Stop,
            key: "db".to_string(),
            method: String::new(),
            framework: "docker-compose".to_string(),
            metadata: None,
        };

        // custom_match requires a real Connection — use a dummy rusqlite in-memory db.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let flows = connector
            .custom_match(&conn, &[start], &[stop])
            .unwrap()
            .unwrap();

        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].edge_type, "service_dependency");
        assert!((flows[0].confidence - 0.9).abs() < f64::EPSILON);
    }
}
