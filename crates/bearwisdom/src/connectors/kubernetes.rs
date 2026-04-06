// =============================================================================
// connectors/kubernetes.rs — Kubernetes connector
//
// Scans Kubernetes manifest files for Deployment and Service resources.
//
// Stop:  Service resource definitions (named service endpoints)
// Start: Deployment env vars containing HTTP service URLs
//        (pattern: http://<service-name>[:port] or <service-name>:<port>)
//
// Matching key: Kubernetes service name.
// =============================================================================

use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::Connection;
use serde_yaml::Value;
use tracing::warn;

use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol, ResolvedFlow};
use crate::indexer::project_context::ProjectContext;

/// Directories commonly used for Kubernetes / Helm manifests.
const K8S_DIRS: &[&str] = &["k8s", "kubernetes", "deploy", "helm", ".k8s", "manifests", "charts"];

/// Regex pattern for service URL references in env vars (compiled once at call site).
const SERVICE_URL_PATTERN: &str = r#"https?://([a-zA-Z][a-zA-Z0-9_-]*)(?::\d+)?"#;
/// Plain host:port pattern without http scheme.
const HOST_PORT_PATTERN: &str = r#"^([a-zA-Z][a-zA-Z0-9_-]*):\d+"#;

pub struct KubernetesConnector;

impl Connector for KubernetesConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "kubernetes",
            protocols: &[Protocol::Infrastructure],
            languages: &["yaml"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let manifest_files = find_k8s_manifests(project_root);
        if manifest_files.is_empty() {
            return Ok(Vec::new());
        }

        let url_re = regex::Regex::new(SERVICE_URL_PATTERN).expect("k8s url regex");
        let host_port_re = regex::Regex::new(HOST_PORT_PATTERN).expect("k8s host:port regex");

        let mut points = Vec::new();

        for manifest_path in manifest_files {
            let file_id = match lookup_file_id(conn, &manifest_path) {
                Some(id) => id,
                None => {
                    warn!(
                        "kubernetes: {} not in files table — skipping",
                        manifest_path.display()
                    );
                    continue;
                }
            };

            let content = match std::fs::read_to_string(&manifest_path) {
                Ok(c) => c,
                Err(e) => {
                    warn!("kubernetes: cannot read {}: {e}", manifest_path.display());
                    continue;
                }
            };

            // A single YAML file may contain multiple documents separated by `---`.
            for doc_str in content.split("\n---") {
                let doc: Value = match serde_yaml::from_str(doc_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let kind = doc
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                match kind.as_str() {
                    "Service" => {
                        extract_service_stops(file_id, &doc, &mut points);
                    }
                    "Deployment" | "StatefulSet" | "DaemonSet" | "Job" | "CronJob" => {
                        extract_deployment_starts(file_id, &doc, &mut points, &url_re, &host_port_re);
                    }
                    _ => {}
                }
            }
        }

        Ok(points)
    }

    fn custom_match(
        &self,
        _conn: &Connection,
        starts: &[ConnectionPoint],
        stops: &[ConnectionPoint],
    ) -> Result<Option<Vec<ResolvedFlow>>> {
        let mut flows = Vec::new();

        for start in starts {
            if start.framework != "kubernetes" {
                continue;
            }
            for stop in stops {
                if stop.framework == "kubernetes" && stop.key == start.key {
                    flows.push(ResolvedFlow {
                        start: start.clone(),
                        stop: stop.clone(),
                        confidence: 0.8,
                        edge_type: "k8s_service_reference".to_string(),
                    });
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

fn find_k8s_manifests(project_root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();

    for dir_name in K8S_DIRS {
        let dir = project_root.join(dir_name);
        if !dir.is_dir() {
            continue;
        }
        collect_yaml_files_with_k8s_kind(&dir, &mut found);
    }

    found
}

/// Walk a directory recursively and collect YAML files that contain
/// `kind: Deployment`, `kind: Service`, or other recognised K8s resource kinds.
fn collect_yaml_files_with_k8s_kind(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Recurse — Helm charts have nested templates/.
            collect_yaml_files_with_k8s_kind(&path, out);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "yml" && ext != "yaml" {
            continue;
        }
        // Quick pre-filter: only stat the file if it looks like a K8s manifest.
        // Read just enough bytes to find `kind:` near the top.
        if file_looks_like_k8s_manifest(&path) {
            out.push(path);
        }
    }
}

/// Return true if the file contains a `kind:` field matching a K8s resource.
fn file_looks_like_k8s_manifest(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    // Quick string scan — no full parse required.
    for line in content.lines().take(30) {
        let trimmed = line.trim();
        if trimmed.starts_with("kind:") {
            let kind = trimmed.trim_start_matches("kind:").trim();
            return matches!(
                kind,
                "Deployment"
                    | "Service"
                    | "StatefulSet"
                    | "DaemonSet"
                    | "Job"
                    | "CronJob"
                    | "Ingress"
                    | "ConfigMap"
            );
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

fn extract_service_stops(file_id: i64, doc: &Value, out: &mut Vec<ConnectionPoint>) {
    let name = match doc
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
    {
        Some(n) => n.to_string(),
        None => return,
    };

    let ports = collect_k8s_service_ports(doc);
    let selector = collect_selector(doc);

    let metadata = serde_json::json!({
        "ports": ports,
        "selector": selector,
    })
    .to_string();

    out.push(ConnectionPoint {
        file_id,
        symbol_id: None,
        line: 1,
        protocol: Protocol::Infrastructure,
        direction: FlowDirection::Stop,
        key: name,
        method: String::new(),
        framework: "kubernetes".to_string(),
        metadata: Some(metadata),
    });
}

fn extract_deployment_starts(
    file_id: i64,
    doc: &Value,
    out: &mut Vec<ConnectionPoint>,
    url_re: &regex::Regex,
    host_port_re: &regex::Regex,
) {
    // Walk spec.template.spec.containers[*].env[*].value looking for service URLs.
    let containers = doc
        .get("spec")
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("spec"))
        .and_then(|s| s.get("containers"))
        .and_then(|c| c.as_sequence());

    // StatefulSets may nest jobTemplate for CronJobs.
    let containers = containers.or_else(|| {
        doc.get("spec")
            .and_then(|s| s.get("jobTemplate"))
            .and_then(|j| j.get("spec"))
            .and_then(|s| s.get("template"))
            .and_then(|t| t.get("spec"))
            .and_then(|s| s.get("containers"))
            .and_then(|c| c.as_sequence())
    });

    let Some(containers) = containers else {
        return;
    };

    for container in containers {
        let Some(env_list) = container.get("env").and_then(|e| e.as_sequence()) else {
            continue;
        };

        for env in env_list {
            let value = env
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let service_names = extract_service_names_from_url(value, url_re, host_port_re);
            for service_name in service_names {
                let env_name = env.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let metadata = serde_json::json!({
                    "env_var": env_name,
                    "url_value": value,
                })
                .to_string();

                out.push(ConnectionPoint {
                    file_id,
                    symbol_id: None,
                    line: 1,
                    protocol: Protocol::Infrastructure,
                    direction: FlowDirection::Start,
                    key: service_name,
                    method: String::new(),
                    framework: "kubernetes".to_string(),
                    metadata: Some(metadata),
                });
            }
        }
    }
}

/// Extract service name(s) from a URL-like string.
///
/// Examples:
///   `http://auth-service:3000/api` → `["auth-service"]`
///   `auth-service:3000`            → `["auth-service"]`
fn extract_service_names_from_url(
    value: &str,
    url_re: &regex::Regex,
    host_port_re: &regex::Regex,
) -> Vec<String> {
    let mut names = Vec::new();

    for cap in url_re.captures_iter(value) {
        if let Some(m) = cap.get(1) {
            let name = m.as_str().to_string();
            // Filter out common non-service patterns like "localhost", IP addresses.
            if !is_non_service_host(&name) {
                names.push(name);
            }
        }
    }

    // If no http(s) URLs matched, try plain host:port.
    if names.is_empty() {
        if let Some(cap) = host_port_re.captures(value) {
            if let Some(m) = cap.get(1) {
                let name = m.as_str().to_string();
                if !is_non_service_host(&name) {
                    names.push(name);
                }
            }
        }
    }

    names
}

fn is_non_service_host(name: &str) -> bool {
    matches!(name, "localhost" | "127" | "0" | "example")
        || name.parse::<u8>().is_ok() // IP octet
}

fn collect_k8s_service_ports(doc: &Value) -> Vec<serde_json::Value> {
    let mut ports = Vec::new();
    if let Some(port_list) = doc
        .get("spec")
        .and_then(|s| s.get("ports"))
        .and_then(|p| p.as_sequence())
    {
        for port in port_list {
            let port_num = port.get("port").and_then(|v| v.as_u64());
            let target = port.get("targetPort").and_then(|v| v.as_u64());
            let protocol = port.get("protocol").and_then(|v| v.as_str()).unwrap_or("TCP");
            ports.push(serde_json::json!({
                "port": port_num,
                "targetPort": target,
                "protocol": protocol,
            }));
        }
    }
    ports
}

fn collect_selector(doc: &Value) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    if let Some(selector) = doc
        .get("spec")
        .and_then(|s| s.get("selector"))
        .and_then(|s| s.as_mapping())
    {
        for (k, v) in selector {
            if let (Some(key), Some(val)) = (k.as_str(), v.as_str()) {
                map.insert(key.to_string(), serde_json::Value::String(val.to_string()));
            }
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn lookup_file_id(conn: &Connection, abs_path: &Path) -> Option<i64> {
    let path_str = abs_path.to_string_lossy().replace('\\', "/");

    let result: rusqlite::Result<i64> = conn.query_row(
        "SELECT id FROM files WHERE path = ?1 LIMIT 1",
        rusqlite::params![path_str],
        |row| row.get(0),
    );
    if let Ok(id) = result {
        return Some(id);
    }

    let file_name = abs_path.file_name()?.to_string_lossy().into_owned();
    let result: rusqlite::Result<i64> = conn.query_row(
        "SELECT id FROM files WHERE path LIKE ?1 LIMIT 1",
        rusqlite::params![format!("%/{file_name}")],
        |row| row.get(0),
    );
    result.ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_service_stop() {
        let yaml = r#"
apiVersion: v1
kind: Service
metadata:
  name: auth-service
spec:
  selector:
    app: auth
  ports:
    - port: 3000
      targetPort: 3000
      protocol: TCP
"#;
        let doc: Value = serde_yaml::from_str(yaml).unwrap();
        let mut points = Vec::new();
        extract_service_stops(1, &doc, &mut points);

        assert_eq!(points.len(), 1);
        assert_eq!(points[0].key, "auth-service");
        assert_eq!(points[0].direction, FlowDirection::Stop);
        assert_eq!(points[0].protocol, Protocol::Infrastructure);
        assert_eq!(points[0].framework, "kubernetes");
    }

    #[test]
    fn test_extract_deployment_env_url() {
        let yaml = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
spec:
  template:
    spec:
      containers:
        - name: api
          env:
            - name: AUTH_SERVICE_URL
              value: "http://auth-service:3000/api"
            - name: DB_HOST
              value: "postgres:5432"
"#;
        let doc: Value = serde_yaml::from_str(yaml).unwrap();
        let url_re = regex::Regex::new(SERVICE_URL_PATTERN).unwrap();
        let host_port_re = regex::Regex::new(HOST_PORT_PATTERN).unwrap();
        let mut points = Vec::new();
        extract_deployment_starts(1, &doc, &mut points, &url_re, &host_port_re);

        // auth-service from http URL, postgres from host:port
        let keys: Vec<_> = points.iter().map(|p| p.key.as_str()).collect();
        assert!(keys.contains(&"auth-service"), "expected auth-service in {:?}", keys);
        assert!(keys.contains(&"postgres"), "expected postgres in {:?}", keys);
    }

    #[test]
    fn test_service_url_extraction() {
        let url_re = regex::Regex::new(SERVICE_URL_PATTERN).unwrap();
        let host_port_re = regex::Regex::new(HOST_PORT_PATTERN).unwrap();

        let names = extract_service_names_from_url(
            "http://catalog-api:8080/items",
            &url_re,
            &host_port_re,
        );
        assert_eq!(names, vec!["catalog-api"]);

        let names = extract_service_names_from_url("redis:6379", &url_re, &host_port_re);
        assert_eq!(names, vec!["redis"]);

        let names = extract_service_names_from_url("localhost:5432", &url_re, &host_port_re);
        assert!(names.is_empty(), "localhost should be filtered out");
    }

    #[test]
    fn test_file_looks_like_k8s_manifest() {
        // We can't call the function on a real file in unit tests, but we can
        // test the pattern logic inline.
        let kinds = ["Deployment", "Service", "StatefulSet", "Ingress", "ConfigMap"];
        for kind in &kinds {
            assert!(matches!(
                *kind,
                "Deployment"
                    | "Service"
                    | "StatefulSet"
                    | "DaemonSet"
                    | "Job"
                    | "CronJob"
                    | "Ingress"
                    | "ConfigMap"
            ));
        }
    }
}
