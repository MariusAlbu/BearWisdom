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

// ===========================================================================
// PythonRestConnector — HTTP client call starts + route stops for Python
// ===========================================================================

pub struct PythonRestConnector;

impl Connector for PythonRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "python_rest",
            protocols: &[Protocol::Rest],
            languages: &["python"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();
        extract_python_rest_stops(conn, &mut points)?;
        extract_python_rest_starts(conn, project_root, &mut points)?;
        Ok(points)
    }
}

fn extract_python_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'python'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare Python REST stops query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<u32>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .context("Failed to query Python routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read Python route row")?;
        out.push(ConnectionPoint {
            file_id,
            symbol_id,
            line: line.unwrap_or(0),
            protocol: Protocol::Rest,
            direction: FlowDirection::Stop,
            key: route,
            method: method.to_uppercase(),
            framework: String::new(),
            metadata: None,
        });
    }
    Ok(())
}

fn extract_python_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // requests.get/post/put/delete/patch + httpx.get/post/…
    let re_requests = regex::Regex::new(
        r#"requests\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    ).expect("python requests regex");
    let re_httpx = regex::Regex::new(
        r#"httpx\s*\.\s*(?P<method>get|post|put|delete|patch)\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    ).expect("python httpx regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'python'")
        .context("Failed to prepare Python files query")?;
    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Python files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Python file rows")?;

    for (file_id, rel_path) in files {
        if rest_is_test_or_config_file(&rel_path) {
            continue;
        }
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for re in &[&re_requests, &re_httpx] {
                for cap in re.captures_iter(line_text) {
                    let raw_url = cap.name("url1")
                        .or_else(|| cap.name("url2"))
                        .map(|m| m.as_str().to_string());
                    let Some(raw_url) = raw_url else { continue };
                    if !rest_looks_like_backend_api_url(&raw_url) {
                        continue;
                    }
                    let method = cap.name("method")
                        .map(|m| m.as_str().to_uppercase())
                        .unwrap_or_else(|| "GET".to_string());
                    let url_pattern = rest_normalise_url_pattern(&raw_url);
                    out.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Start,
                        key: url_pattern,
                        method,
                        framework: String::new(),
                        metadata: None,
                    });
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared REST detection helpers
// ---------------------------------------------------------------------------

fn rest_is_test_or_config_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.contains("_test.") || lower.contains(".test.")
        || lower.contains("test/") || lower.contains("/tests/")
}

fn rest_looks_like_backend_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() { return false; }
        return rest_looks_like_backend_api_url(path);
    }
    let lower = s.to_lowercase();
    if let Some(last_seg) = lower.rsplit('/').next() {
        if last_seg.contains('.') {
            let ext = lower.rsplit('.').next().unwrap_or("");
            if matches!(ext, "svg"|"png"|"jpg"|"jpeg"|"gif"|"ico"|"webp"|"css"|"js"|"html"|"htm"|"xml"|"json"|"txt"|"md"|"pdf") {
                return false;
            }
        }
    }
    s.starts_with('/') || s.contains("/api/") || s.contains("/v1/") || s.contains("/v2/") || s.contains("/v3/") || s.contains("/${") || s.contains("/{")
}

fn rest_normalise_url_pattern(raw: &str) -> String {
    let without_query = raw.split('?').next().unwrap_or(raw);
    let re_tmpl = regex::Regex::new(r"\$\{[^}]+\}").expect("template regex");
    re_tmpl.replace_all(without_query, "{param}").into_owned()
}

// ===========================================================================
// Django model/view concept post-index hook
// ===========================================================================

/// Detect Django models and views and write flow_edges for them.
///
/// Called from `PythonPlugin::post_index()` when Django is detected.
/// The URL/route detection is handled separately by `DjangoRouteConnector`.
pub fn run_django_concepts(db: &crate::db::Database, project_root: &std::path::Path) {
    use tracing::warn;

    match detect_django_models(db.conn(), project_root) {
        Ok(n) if n > 0 => tracing::info!(n, "Django models detected"),
        Err(e) => warn!("Django model detection: {e}"),
        _ => {}
    }
    match detect_django_views(db.conn(), project_root) {
        Ok(n) if n > 0 => tracing::info!(n, "Django views detected"),
        Err(e) => warn!("Django view detection: {e}"),
        _ => {}
    }
}

fn detect_django_models(
    conn: &rusqlite::Connection,
    project_root: &std::path::Path,
) -> anyhow::Result<u32> {
    let re_model = regex::Regex::new(r"class\s+(\w+)\s*\(\s*models\.Model\s*\)")
        .expect("django model regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'python'")
        .context("prepare Python files for model scan")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("query Python files for model scan")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect Python file rows for model scan")?;

    let mut count: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re_model.captures_iter(line_text) {
                let class_name = &cap[1];
                let result = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, protocol, confidence
                     ) VALUES (
                        ?1, ?2, ?3, 'python',
                        ?1, ?2, ?3, 'python',
                        'django_model', 'orm', 0.95
                     )",
                    rusqlite::params![file_id, line_no, class_name],
                );
                if result.map(|n| n > 0).unwrap_or(false) {
                    count += 1;
                }
            }
        }
    }

    Ok(count)
}

fn detect_django_views(
    conn: &rusqlite::Connection,
    project_root: &std::path::Path,
) -> anyhow::Result<u32> {
    let re_cbv = regex::Regex::new(r"class\s+(\w+)\s*\([^)]*View[^)]*\)")
        .expect("django cbv regex");
    let re_fbv = regex::Regex::new(r"def\s+(\w+)\s*\(\s*request")
        .expect("django fbv regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'python'")
        .context("prepare Python files for view scan")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("query Python files for view scan")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collect Python file rows for view scan")?;

    let mut count: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;

            for cap in re_cbv.captures_iter(line_text) {
                let class_name = &cap[1];
                let result = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, protocol, confidence
                     ) VALUES (
                        ?1, ?2, ?3, 'python',
                        ?1, ?2, ?3, 'python',
                        'django_view', 'http', 0.90
                     )",
                    rusqlite::params![file_id, line_no, class_name],
                );
                if result.map(|n| n > 0).unwrap_or(false) {
                    count += 1;
                }
            }

            for cap in re_fbv.captures_iter(line_text) {
                let fn_name = &cap[1];
                if fn_name.starts_with("test_") {
                    continue;
                }
                let result = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, protocol, confidence
                     ) VALUES (
                        ?1, ?2, ?3, 'python',
                        ?1, ?2, ?3, 'python',
                        'django_view', 'http', 0.85
                     )",
                    rusqlite::params![file_id, line_no, fn_name],
                );
                if result.map(|n| n > 0).unwrap_or(false) {
                    count += 1;
                }
            }
        }
    }

    Ok(count)
}

// ===========================================================================
// PythonGrpcConnector — gRPC service implementation stops
// ===========================================================================

/// Detects Python gRPC service implementations generated by grpcio-tools.
///
/// The generated base class is `{ServiceName}Servicer` (in the `*_pb2_grpc.py`
/// file).  Implementations subclass it and override the RPC methods.
pub struct PythonGrpcConnector;

impl Connector for PythonGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "python_grpc_stops",
            protocols: &[Protocol::Grpc],
            languages: &["python"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.python_packages.contains("grpcio")
            || ctx.python_packages.contains("grpc")
            || ctx.python_packages.contains("grpcio-tools")
    }

    fn extract(&self, conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Find Python classes that inherit from *Servicer (gRPC generated base).
        let mut stmt = conn
            .prepare(
                "SELECT s.name, s.file_id
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.language = 'python'
                   AND s.kind = 'class'
                   AND EXISTS (
                       SELECT 1 FROM edges e
                       JOIN symbols tgt ON tgt.id = e.target_id
                       WHERE e.source_id = s.id
                         AND e.kind = 'inherits'
                         AND tgt.name LIKE '%Servicer'
                   )",
            )
            .context("Failed to prepare Python gRPC class query")?;

        let classes: Vec<(String, i64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .context("Failed to query Python gRPC classes")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Python gRPC class rows")?;

        let mut points = Vec::new();

        for (_class_name, file_id) in &classes {
            let parent_name: Option<String> = conn
                .query_row(
                    "SELECT tgt.name FROM edges e
                     JOIN symbols src ON src.id = e.source_id
                     JOIN symbols tgt ON tgt.id = e.target_id
                     WHERE src.file_id = ?1
                       AND e.kind = 'inherits'
                       AND tgt.name LIKE '%Servicer'
                     LIMIT 1",
                    rusqlite::params![file_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();

            let service_name = parent_name
                .as_deref()
                .and_then(|n| n.strip_suffix("Servicer"))
                .unwrap_or("")
                .to_string();

            if service_name.is_empty() {
                continue;
            }

            let mut method_stmt = conn
                .prepare(
                    "SELECT s.id, s.name, s.line
                     FROM symbols s
                     WHERE s.file_id = ?1 AND s.kind = 'method'",
                )
                .context("Failed to prepare Python gRPC method query")?;

            let methods: Vec<(i64, String, u32)> = method_stmt
                .query_map(rusqlite::params![file_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                })
                .context("Failed to query Python gRPC methods")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect Python gRPC method rows")?;

            for (sym_id, method_name, line) in methods {
                if method_name.starts_with("__") {
                    continue;
                }
                let key = format!("{service_name}.{method_name}");
                points.push(ConnectionPoint {
                    file_id: *file_id,
                    symbol_id: Some(sym_id),
                    line,
                    protocol: Protocol::Grpc,
                    direction: FlowDirection::Stop,
                    key,
                    method: String::new(),
                    framework: "grpcio".to_string(),
                    metadata: None,
                });
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// PythonMqConnector — Message queue producer/consumer stops
// ===========================================================================

/// Detects Python message queue patterns:
///   - Celery: `@app.task` / `@celery.task` / `@shared_task` (consumer)
///   - kafka-python / confluent-kafka: `producer.send("topic")` (producer)
///                                     `consumer.subscribe(["topic"])` (consumer)
///   - pika (RabbitMQ): `channel.basic_publish(routing_key="key")` (producer)
///                       `channel.basic_consume("queue", ...)` (consumer)
pub struct PythonMqConnector;

impl Connector for PythonMqConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "python_mq",
            protocols: &[Protocol::MessageQueue],
            languages: &["python"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.python_packages.contains("celery")
            || ctx.python_packages.contains("kafka-python")
            || ctx.python_packages.contains("confluent-kafka")
            || ctx.python_packages.contains("pika")
            || ctx.python_packages.contains("aio-pika")
            || ctx.python_packages.contains("kombu")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let re_celery_task = regex::Regex::new(
            r#"@(?:\w+\.)?(?:task|shared_task)\s*(?:\([^)]*\))?\s*$"#,
        )
        .expect("python celery task regex");

        let re_producer_send = regex::Regex::new(
            r#"producer\.send\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("python producer send regex");

        let re_consumer_subscribe = regex::Regex::new(
            r#"consumer\.subscribe\s*\(\s*\[?\s*['"]([^'"]+)['"]"#,
        )
        .expect("python consumer subscribe regex");

        let re_rabbit_publish = regex::Regex::new(
            r#"channel\.basic_publish\s*\([^)]*routing_key\s*=\s*['"]([^'"]+)['"]"#,
        )
        .expect("python rabbit publish regex");

        let re_rabbit_consume = regex::Regex::new(
            r#"channel\.basic_consume\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("python rabbit consume regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'python'")
            .context("Failed to prepare Python files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
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

            let lines: Vec<&str> = source.lines().collect();

            for (line_idx, line_text) in lines.iter().enumerate() {
                let line_no = (line_idx + 1) as u32;

                if re_celery_task.is_match(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: "celery_task".to_string(),
                        method: String::new(),
                        framework: "celery".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_producer_send.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "kafka".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_consumer_subscribe.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "kafka".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_rabbit_publish.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_rabbit_consume.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// PythonGraphQlConnector — GraphQL resolver stops
// ===========================================================================

/// Detects Python GraphQL resolvers for Strawberry, Ariadne, and Graphene.
///
/// Start points come from .graphql schema files (graphql language plugin).
/// This connector emits Stop points for decorated resolvers and Graphene
/// `resolve_*` method conventions.
pub struct PythonGraphQlConnector;

impl Connector for PythonGraphQlConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "python_graphql_resolvers",
            protocols: &[Protocol::GraphQl],
            languages: &["python"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.python_packages.contains("strawberry-graphql")
            || ctx.python_packages.contains("ariadne")
            || ctx.python_packages.contains("graphene")
            || ctx.python_packages.contains("graphql-core")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let re_ariadne_field = regex::Regex::new(
            r#"@(?:query|mutation|subscription)\.field\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("python ariadne field regex");

        let re_strawberry = regex::Regex::new(
            r#"@strawberry\.(?:field|mutation|query|subscription)\s*(?:\([^)]*\))?\s*$"#,
        )
        .expect("python strawberry field regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'python'")
            .context("Failed to prepare Python files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
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

            let lines: Vec<&str> = source.lines().collect();

            for (line_idx, line_text) in lines.iter().enumerate() {
                let line_no = (line_idx + 1) as u32;

                for cap in re_ariadne_field.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::GraphQl,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "ariadne".to_string(),
                        metadata: None,
                    });
                }

                if re_strawberry.is_match(line_text) {
                    let fn_name = lines
                        .iter()
                        .skip(line_idx + 1)
                        .find(|l| {
                            let t = l.trim();
                            !t.is_empty() && !t.starts_with('@') && !t.starts_with('#')
                        })
                        .and_then(|l| {
                            let t = l.trim();
                            t.strip_prefix("async ")
                                .unwrap_or(t)
                                .strip_prefix("def ")
                                .and_then(|s| s.split('(').next())
                                .map(str::trim)
                                .map(str::to_string)
                        });

                    if let Some(name) = fn_name {
                        points.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::GraphQl,
                            direction: FlowDirection::Stop,
                            key: name,
                            method: String::new(),
                            framework: "strawberry".to_string(),
                            metadata: None,
                        });
                    }
                }
            }
        }

        // Graphene-style resolve_* methods.
        let mut resolve_stmt = conn
            .prepare(
                "SELECT s.id, s.name, s.file_id, s.line
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.language = 'python'
                   AND s.kind = 'method'
                   AND s.name LIKE 'resolve_%'",
            )
            .context("Failed to prepare Graphene resolver query")?;

        let resolve_rows: Vec<(i64, String, i64, u32)> = resolve_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .context("Failed to query Graphene resolvers")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Graphene resolver rows")?;

        for (sym_id, name, file_id, line) in resolve_rows {
            let field_name = name
                .strip_prefix("resolve_")
                .unwrap_or(name.as_str())
                .to_string();

            points.push(ConnectionPoint {
                file_id,
                symbol_id: Some(sym_id),
                line,
                protocol: Protocol::GraphQl,
                direction: FlowDirection::Stop,
                key: field_name,
                method: String::new(),
                framework: "graphene".to_string(),
                metadata: None,
            });
        }

        Ok(points)
    }
}
