// =============================================================================
// languages/python/connectors.rs — Python language plugin connectors
//
// Django and FastAPI route connectors, migrated from connectors/route_connectors.rs.
// These are returned by PythonPlugin::connectors() and registered into the
// ConnectorRegistry alongside other cross-cutting connectors.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// Django
// ===========================================================================

pub struct DjangoRouteConnector;

impl Connector for DjangoRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "django_routes",
            protocols: &[Protocol::Rest],
            languages: &["python"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.python_packages.contains("django")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let re_url = regex::Regex::new(
            r#"(?:re_)?path\s*\(\s*r?['"]([^'"]+)['"]\s*,\s*(\w[\w.]*)"#,
        )
        .expect("django url regex");
        // DRF: router.register(r"prefix", ViewSetClass) or router.register("prefix", ViewSetClass)
        let re_router = regex::Regex::new(
            r#"\w+\.register\s*\(\s*r?['"]([^'"]+)['"]\s*,\s*(\w[\w.]*)"#,
        )
        .expect("drf router regex");

        let mut stmt = conn
            .prepare(
                "SELECT id, path FROM files
                 WHERE language = 'python' AND (path LIKE '%urls.py' OR path = 'urls.py')",
            )
            .context("Failed to prepare Django urls query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query Django url files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Django url files")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                // path() / re_path() patterns
                for cap in re_url.captures_iter(line_text) {
                    let route_path = cap[1].to_string();
                    let view_ref = &cap[2];
                    let view_name = view_ref.split('.').next_back().unwrap_or(view_ref);

                    let symbol_id: Option<i64> = conn
                        .query_row(
                            "SELECT s.id FROM symbols s
                             JOIN files f ON f.id = s.file_id
                             WHERE s.name = ?1 AND f.language = 'python'
                               AND s.kind IN ('function', 'class', 'method')
                             LIMIT 1",
                            rusqlite::params![view_name],
                            |r| r.get(0),
                        )
                        .ok();

                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Stop,
                        key: route_path,
                        method: "GET".to_string(),
                        framework: "django".to_string(),
                        metadata: None,
                    });
                }

                // DRF router.register(r"prefix", ViewSetClass)
                for cap in re_router.captures_iter(line_text) {
                    let prefix = format!("/{}", cap[1].trim_start_matches('/'));
                    let viewset = cap[2].to_string();

                    let symbol_id: Option<i64> = conn
                        .query_row(
                            "SELECT s.id FROM symbols s
                             JOIN files f ON f.id = s.file_id
                             WHERE s.name = ?1 AND f.language = 'python'
                               AND s.kind = 'class'
                             LIMIT 1",
                            rusqlite::params![viewset],
                            |r| r.get(0),
                        )
                        .ok();

                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Stop,
                        key: prefix,
                        method: "GET".to_string(),
                        framework: "django".to_string(),
                        metadata: None,
                    });
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// FastAPI / Starlette
// ===========================================================================

pub struct FastApiRouteConnector;

impl Connector for FastApiRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "fastapi_routes",
            protocols: &[Protocol::Rest],
            languages: &["python"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.python_packages.contains("fastapi") || ctx.python_packages.contains("starlette")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let re_decorator = regex::Regex::new(
            r#"@(\w+)\.(get|post|put|delete|patch|head|options)\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("fastapi decorator regex");
        let re_apirouter = regex::Regex::new(
            r#"(\w+)\s*=\s*APIRouter\s*\([^)]*prefix\s*=\s*['"]([^'"]*)['"]\s*[,)]"#,
        )
        .expect("fastapi APIRouter regex");
        let re_include = regex::Regex::new(
            r#"include_router\s*\(\s*(\w+)(?:[^)]*prefix\s*=\s*['"]([^'"]*)['"]\s*)?[,)]"#,
        )
        .expect("fastapi include_router regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'python'")
            .context("Failed to prepare Python files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query Python files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Python file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let prefixes = collect_prefixes(&source, &re_apirouter, &re_include);

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                if let Some(cap) = re_decorator.captures(line_text) {
                    let var_name = &cap[1];
                    let http_method = cap[2].to_uppercase();
                    let route_path = &cap[3];

                    let prefix = prefixes.get(var_name).map(|s| s.as_str()).unwrap_or("");
                    let resolved = join_prefix(prefix, route_path);

                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Stop,
                        key: resolved,
                        method: http_method,
                        framework: "fastapi".to_string(),
                        metadata: None,
                    });
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Join a prefix and a path, ensuring exactly one `/` between them.
fn join_prefix(prefix: &str, path: &str) -> String {
    match (prefix.trim_end_matches('/'), path.trim_start_matches('/')) {
        ("", p) => format!("/{p}"),
        (pre, "") => pre.to_owned(),
        (pre, p) => format!("{pre}/{p}"),
    }
}

/// Build a map of `variable_name → effective_prefix` for a single file's source.
///
/// Two sources of prefix:
///   - `router = APIRouter(prefix="/users")` — declared in this file
///   - `app.include_router(router, prefix="/api/v1")` — mount override
///
/// When both are present the prefixes are concatenated.
fn collect_prefixes(
    source: &str,
    re_apirouter: &regex::Regex,
    re_include: &regex::Regex,
) -> HashMap<String, String> {
    let mut declared: HashMap<String, String> = HashMap::new();
    let mut mounted: HashMap<String, String> = HashMap::new();

    for line in source.lines() {
        if let Some(cap) = re_apirouter.captures(line) {
            declared.insert(cap[1].to_owned(), cap[2].to_owned());
        }
        if let Some(cap) = re_include.captures(line) {
            let mount_prefix = cap.get(2).map(|m| m.as_str()).unwrap_or("").to_owned();
            if !mount_prefix.is_empty() {
                mounted.insert(cap[1].to_owned(), mount_prefix);
            }
        }
    }

    // Merge: effective prefix = mount_prefix + declared_prefix
    let mut result: HashMap<String, String> = declared.clone();
    for (var, mount) in &mounted {
        let declared_part = declared.get(var).map(|s| s.as_str()).unwrap_or("");
        result.insert(var.clone(), join_prefix(mount, declared_part));
    }
    for (var, mount) in &mounted {
        result.entry(var.clone()).or_insert_with(|| mount.clone());
    }

    result
}
