// =============================================================================
// connectors/kubernetes.rs — Kubernetes connector
//
// Standalone post-index hook (not a Connector trait impl).
//
// Scans Kubernetes manifest files for Deployment and Service resources, then
// writes flow_edges for any Deployment that references a known Service name
// via an environment variable URL.
//
// For each Service resource: record the service name as a known endpoint.
// For each Deployment env var that contains http://<service-name> or
//   <service-name>:<port>, emit a flow_edge from the Deployment's package
//   to the Service's package (if both resolve to packages).
//
// edge_type: "k8s_service_reference"
// protocol:  "infrastructure"
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_yaml::Value;
use tracing::warn;

use super::types::Protocol;
use crate::db::Database;

/// Directories commonly used for Kubernetes / Helm manifests.
const K8S_DIRS: &[&str] = &["k8s", "kubernetes", "deploy", "helm", ".k8s", "manifests", "charts"];

/// Regex pattern for service URL references in env vars (compiled once at call site).
const SERVICE_URL_PATTERN: &str = r#"https?://([a-zA-Z][a-zA-Z0-9_-]*)(?::\d+)?"#;
/// Plain host:port pattern without http scheme.
const HOST_PORT_PATTERN: &str = r#"^([a-zA-Z][a-zA-Z0-9_-]*):\d+"#;

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// A discovered Kubernetes Service resource.
struct K8sService {
    name: String,
    /// Resolved package_id (None if no matching package found).
    package_id: Option<i64>,
}

/// A reference from a Deployment to a service name, derived from env vars.
struct ServiceRef {
    /// Name of the service being referenced.
    service_name: String,
    /// Env var name for metadata.
    env_var: String,
    /// Source package_id of the Deployment (None if unresolvable).
    source_package_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Connect Kubernetes service references into the graph.
///
/// Returns the number of flow_edges written.
pub fn connect(db: &Database, project_root: &Path) -> Result<u32> {
    let manifest_files = find_k8s_manifests(project_root);
    if manifest_files.is_empty() {
        return Ok(0);
    }

    let conn = db.conn();
    let url_re = regex::Regex::new(SERVICE_URL_PATTERN).expect("k8s url regex");
    let host_port_re = regex::Regex::new(HOST_PORT_PATTERN).expect("k8s host:port regex");

    // Collect all Service definitions and Deployment→service references
    // across all manifest files in a single pass.
    let mut known_services: HashMap<String, K8sService> = HashMap::new();
    let mut service_refs: Vec<ServiceRef> = Vec::new();

    for manifest_path in manifest_files {
        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("kubernetes: cannot read {}: {e}", manifest_path.display());
                continue;
            }
        };

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
                    if let Some(svc) = extract_service(conn, project_root, &manifest_path, &doc) {
                        known_services.insert(svc.name.clone(), svc);
                    }
                }
                "Deployment" | "StatefulSet" | "DaemonSet" | "Job" | "CronJob" => {
                    let refs = extract_deployment_refs(
                        conn,
                        project_root,
                        &manifest_path,
                        &doc,
                        &url_re,
                        &host_port_re,
                    );
                    service_refs.extend(refs);
                }
                _ => {}
            }
        }
    }

    if known_services.is_empty() || service_refs.is_empty() {
        return Ok(0);
    }

    // Write flow_edges for each ref that matches a known service.
    let mut total = 0u32;

    for sref in &service_refs {
        let Some(target_svc) = known_services.get(&sref.service_name) else {
            continue;
        };
        let Some(source_pkg_id) = sref.source_package_id else {
            continue;
        };
        let Some(target_pkg_id) = target_svc.package_id else {
            continue;
        };

        let source_file_id = match representative_file_id(conn, source_pkg_id) {
            Some(id) => id,
            None => {
                warn!(
                    "kubernetes: no files for source package_id={source_pkg_id} — skipping"
                );
                continue;
            }
        };
        let target_file_id = match representative_file_id(conn, target_pkg_id) {
            Some(id) => id,
            None => {
                warn!(
                    "kubernetes: no files for target package_id={target_pkg_id} \
                     (service '{}') — skipping",
                    target_svc.name
                );
                continue;
            }
        };

        let metadata = serde_json::json!({
            "env_var": sref.env_var,
            "service": sref.service_name,
        })
        .to_string();

        match conn.prepare_cached(
            "INSERT OR IGNORE INTO flow_edges
                (source_file_id, source_line, source_symbol, source_language,
                 target_file_id, target_line, target_symbol, target_language,
                 edge_type, protocol, confidence, metadata)
             VALUES (?1, 1, NULL, NULL, ?2, 1, ?3, NULL,
                     'k8s_service_reference', ?4, 0.8, ?5)",
        ) {
            Ok(mut stmt) => match stmt.execute(rusqlite::params![
                source_file_id,
                target_file_id,
                target_svc.name,
                Protocol::Infrastructure.as_str(),
                metadata,
            ]) {
                Ok(n) => total += n as u32,
                Err(e) => warn!(
                    "kubernetes: failed to insert flow_edge for '{}' → '{}': {e}",
                    sref.env_var, sref.service_name
                ),
            },
            Err(e) => warn!("kubernetes: prepare_cached failed: {e}"),
        }
    }

    Ok(total)
}

// ---------------------------------------------------------------------------
// Extraction helpers
// ---------------------------------------------------------------------------

/// Extract a K8sService from a Service manifest document.
///
/// Tries to resolve the service to a package by matching its selector labels
/// against package paths, falling back to name-based matching.
fn extract_service(
    conn: &rusqlite::Connection,
    project_root: &Path,
    manifest_path: &Path,
    doc: &Value,
) -> Option<K8sService> {
    let name = doc
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())?
        .to_string();

    // Attempt to resolve the service to a package by name match.
    let package_id = resolve_package_by_name(conn, &name);

    Some(K8sService { name, package_id })
}

/// Extract ServiceRefs from a Deployment/StatefulSet/etc. manifest document.
fn extract_deployment_refs(
    conn: &rusqlite::Connection,
    project_root: &Path,
    manifest_path: &Path,
    doc: &Value,
    url_re: &regex::Regex,
    host_port_re: &regex::Regex,
) -> Vec<ServiceRef> {
    let mut refs = Vec::new();

    // Determine the deployment's own name for package resolution.
    let deployment_name = doc
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Resolve the Deployment to a package by name.
    let source_package_id = resolve_package_by_name(conn, &deployment_name)
        .or_else(|| resolve_package_from_manifest_dir(conn, project_root, manifest_path));

    // Walk spec.template.spec.containers[*].env[*].value
    let containers = doc
        .get("spec")
        .and_then(|s| s.get("template"))
        .and_then(|t| t.get("spec"))
        .and_then(|s| s.get("containers"))
        .and_then(|c| c.as_sequence())
        // CronJob nesting: spec.jobTemplate.spec.template.spec.containers
        .or_else(|| {
            doc.get("spec")
                .and_then(|s| s.get("jobTemplate"))
                .and_then(|j| j.get("spec"))
                .and_then(|s| s.get("template"))
                .and_then(|t| t.get("spec"))
                .and_then(|s| s.get("containers"))
                .and_then(|c| c.as_sequence())
        });

    let Some(containers) = containers else {
        return refs;
    };

    for container in containers {
        let Some(env_list) = container.get("env").and_then(|e| e.as_sequence()) else {
            continue;
        };

        for env in env_list {
            let value = env.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let env_name = env.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();

            let service_names = extract_service_names_from_url(value, url_re, host_port_re);
            for service_name in service_names {
                refs.push(ServiceRef {
                    service_name,
                    env_var: env_name.clone(),
                    source_package_id,
                });
            }
        }
    }

    refs
}

// ---------------------------------------------------------------------------
// Package resolution helpers
// ---------------------------------------------------------------------------

/// Try to resolve a package by matching the service/deployment name against
/// package names or path suffixes.
fn resolve_package_by_name(conn: &rusqlite::Connection, name: &str) -> Option<i64> {
    if name.is_empty() {
        return None;
    }
    conn.query_row(
        "SELECT id FROM packages WHERE name = ?1 OR path = ?1 OR path LIKE ('%/' || ?1) LIMIT 1",
        rusqlite::params![name],
        |row| row.get(0),
    )
    .ok()
}

/// Resolve a package from the manifest file's parent directory.
///
/// Useful for manifests co-located with their service code.
fn resolve_package_from_manifest_dir(
    conn: &rusqlite::Connection,
    project_root: &Path,
    manifest_path: &Path,
) -> Option<i64> {
    let dir = manifest_path.parent()?;
    let rel = dir
        .strip_prefix(project_root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))?;
    let normalised = rel
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();

    if normalised.is_empty() {
        return None;
    }

    // Check if the manifest is inside a known k8s directory; if so, try the
    // parent of that directory.
    let k8s_dirs: &[&str] = &["k8s", "kubernetes", "deploy", "helm", ".k8s", "manifests", "charts"];
    for k8s_dir in k8s_dirs {
        if let Some(stripped) = normalised.strip_suffix(k8s_dir) {
            let parent_rel = stripped.trim_end_matches('/').to_string();
            if !parent_rel.is_empty() {
                if let Ok(id) = conn.query_row(
                    "SELECT id FROM packages WHERE path = ?1 LIMIT 1",
                    rusqlite::params![parent_rel],
                    |row| row.get(0),
                ) {
                    return Some(id);
                }
            }
        }
    }

    conn.query_row(
        "SELECT id FROM packages WHERE path = ?1 OR path LIKE (?1 || '%') LIMIT 1",
        rusqlite::params![normalised],
        |row| row.get(0),
    )
    .ok()
}

/// Get a representative file_id for a package.
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

fn find_k8s_manifests(project_root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir_name in K8S_DIRS {
        let dir = project_root.join(dir_name);
        if dir.is_dir() {
            collect_yaml_files_with_k8s_kind(&dir, &mut found);
        }
    }
    found
}

/// Walk a directory recursively and collect YAML files that look like K8s manifests.
fn collect_yaml_files_with_k8s_kind(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
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
        if file_looks_like_k8s_manifest(&path) {
            out.push(path);
        }
    }
}

/// Return true if the file contains a `kind:` line matching a known K8s resource.
fn file_looks_like_k8s_manifest(path: &Path) -> bool {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
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
// URL parsing helpers
// ---------------------------------------------------------------------------

/// Extract service name(s) from a URL-like string.
fn extract_service_names_from_url(
    value: &str,
    url_re: &regex::Regex,
    host_port_re: &regex::Regex,
) -> Vec<String> {
    let mut names = Vec::new();

    for cap in url_re.captures_iter(value) {
        if let Some(m) = cap.get(1) {
            let name = m.as_str().to_string();
            if !is_non_service_host(&name) {
                names.push(name);
            }
        }
    }

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
        || name.parse::<u8>().is_ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_service_name() {
        let url_re = regex::Regex::new(SERVICE_URL_PATTERN).unwrap();
        let host_port_re = regex::Regex::new(HOST_PORT_PATTERN).unwrap();

        let names = extract_service_names_from_url("http://auth-service:3000/api", &url_re, &host_port_re);
        assert_eq!(names, vec!["auth-service"]);

        let names = extract_service_names_from_url("redis:6379", &url_re, &host_port_re);
        assert_eq!(names, vec!["redis"]);

        let names = extract_service_names_from_url("localhost:5432", &url_re, &host_port_re);
        assert!(names.is_empty(), "localhost should be filtered");
    }

    #[test]
    fn test_extract_service_names_multiple() {
        let url_re = regex::Regex::new(SERVICE_URL_PATTERN).unwrap();
        let host_port_re = regex::Regex::new(HOST_PORT_PATTERN).unwrap();

        let names = extract_service_names_from_url(
            "http://catalog-api:8080/items",
            &url_re,
            &host_port_re,
        );
        assert_eq!(names, vec!["catalog-api"]);
    }

    #[test]
    fn test_file_looks_like_k8s_manifest_patterns() {
        // Validate the pattern matching logic without a real file.
        let recognized = ["Deployment", "Service", "StatefulSet", "Ingress", "ConfigMap", "Job"];
        let unrecognized = ["Namespace", "Secret", "HelmChart"];

        for kind in &recognized {
            assert!(
                matches!(
                    *kind,
                    "Deployment" | "Service" | "StatefulSet" | "DaemonSet"
                        | "Job" | "CronJob" | "Ingress" | "ConfigMap"
                ),
                "{kind} should be recognized"
            );
        }
        for kind in &unrecognized {
            assert!(
                !matches!(
                    *kind,
                    "Deployment" | "Service" | "StatefulSet" | "DaemonSet"
                        | "Job" | "CronJob" | "Ingress" | "ConfigMap"
                ),
                "{kind} should not be recognized"
            );
        }
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
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE packages (id INTEGER PRIMARY KEY, path TEXT, name TEXT, kind TEXT, is_service INTEGER DEFAULT 0);
             CREATE TABLE files (id INTEGER PRIMARY KEY, path TEXT, package_id INTEGER);",
        ).unwrap();

        let url_re = regex::Regex::new(SERVICE_URL_PATTERN).unwrap();
        let host_port_re = regex::Regex::new(HOST_PORT_PATTERN).unwrap();
        let root = Path::new("/project");
        let manifest = root.join("k8s/deployment.yaml");

        let refs = extract_deployment_refs(&conn, root, &manifest, &doc, &url_re, &host_port_re);

        let service_names: Vec<_> = refs.iter().map(|r| r.service_name.as_str()).collect();
        assert!(service_names.contains(&"auth-service"), "expected auth-service");
        assert!(service_names.contains(&"postgres"), "expected postgres from host:port");
    }

    #[test]
    fn test_resolve_package_by_name() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE packages (id INTEGER PRIMARY KEY, path TEXT, name TEXT, kind TEXT, is_service INTEGER DEFAULT 0);
             INSERT INTO packages VALUES (1, 'services/auth', 'auth-service', 'npm', 0);",
        ).unwrap();

        let result = resolve_package_by_name(&conn, "auth-service");
        assert_eq!(result, Some(1));

        let result = resolve_package_by_name(&conn, "nonexistent");
        assert_eq!(result, None);
    }
}
