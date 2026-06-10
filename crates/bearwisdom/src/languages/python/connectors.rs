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

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// Django
// ===========================================================================

/// Django urls.py scan + DRF router pattern detection. Writes detected routes
/// to the `routes` table; the routes-table → FlowEmission bridge in
/// resolve/mod.rs handles downstream flow_edges emission.
///
/// Returns the count of routes written to the `routes` table.
pub fn discover_django_routes(conn: &Connection, project_root: &Path, ctx: &ProjectContext) -> u32 {
    if !ctx.has_dependency(ManifestKind::PyProject, "django") {
        return 0;
    }
    match _django_routes_inner(conn, project_root) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("Django route detection failed: {e}");
            0
        }
    }
}

fn _django_routes_inner(conn: &Connection, project_root: &Path) -> Result<u32> {
    let re_url = regex::Regex::new(r#"(?:re_)?path\s*\(\s*r?['"]([^'"]+)['"]\s*,\s*(\w[\w.]*)"#)
        .expect("django url regex");
    // DRF: router.register(r"prefix", ViewSetClass) or router.register("prefix", ViewSetClass)
    let re_router =
        regex::Regex::new(r#"\w+\.register\s*\(\s*r?['"]([^'"]+)['"]\s*,\s*(\w[\w.]*)"#)
            .expect("drf router regex");

    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
                 WHERE language = 'python' AND (path LIKE '%urls.py' OR path = 'urls.py')",
        )
        .context("Failed to prepare Django urls query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .context("Failed to query Django url files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Django url files")?;

    let mut inserted: u32 = 0;

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

                if insert_python_route(conn, file_id, symbol_id, "GET", &route_path, line_no) {
                    inserted += 1;
                }
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

                if insert_python_route(conn, file_id, symbol_id, "GET", &prefix, line_no) {
                    inserted += 1;
                }
            }
        }
    }

    Ok(inserted)
}

/// Insert a single Python-discovered route row. Returns true on insert.
fn insert_python_route(
    conn: &Connection,
    file_id: i64,
    symbol_id: Option<i64>,
    http_method: &str,
    route: &str,
    line: u32,
) -> bool {
    let result = conn.execute(
        "INSERT OR IGNORE INTO routes
           (file_id, symbol_id, http_method, route_template, resolved_route, line)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
        rusqlite::params![file_id, symbol_id, http_method, route, line],
    );
    matches!(result, Ok(n) if n > 0)
}

// ===========================================================================
// FastAPI / Starlette
// ===========================================================================

/// FastAPI/Starlette decorator + APIRouter prefix join. Writes detected
/// routes to the `routes` table; the routes-table → FlowEmission bridge in
/// resolve/mod.rs handles downstream flow_edges emission.
///
/// Returns the count of routes written to the `routes` table.
pub fn discover_fastapi_routes(
    conn: &Connection,
    project_root: &Path,
    ctx: &ProjectContext,
) -> u32 {
    if !ctx.has_dependency(ManifestKind::PyProject, "fastapi")
        && !ctx.has_dependency(ManifestKind::PyProject, "starlette")
    {
        return 0;
    }
    match _fastapi_routes_inner(conn, project_root) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("FastAPI route detection failed: {e}");
            0
        }
    }
}

fn _fastapi_routes_inner(conn: &Connection, project_root: &Path) -> Result<u32> {
    let re_decorator = regex::Regex::new(
        r#"@(\w+)\.(get|post|put|delete|patch|head|options)\s*\(\s*['"]([^'"]+)['"]"#,
    )
    .expect("fastapi decorator regex");
    let re_apirouter =
        regex::Regex::new(r#"(\w+)\s*=\s*APIRouter\s*\([^)]*prefix\s*=\s*['"]([^'"]*)['"]\s*[,)]"#)
            .expect("fastapi APIRouter regex");
    let re_include = regex::Regex::new(
        r#"include_router\s*\(\s*(\w+)(?:[^)]*prefix\s*=\s*['"]([^'"]*)['"]\s*)?[,)]"#,
    )
    .expect("fastapi include_router regex");

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

    let mut inserted: u32 = 0;

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

                if insert_python_route(conn, file_id, None, &http_method, &resolved, line_no) {
                    inserted += 1;
                }
            }
        }
    }

    Ok(inserted)
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
    let re_model =
        regex::Regex::new(r"class\s+(\w+)\s*\(\s*models\.Model\s*\)").expect("django model regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'python'")
        .context("prepare Python files for model scan")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
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
    let re_cbv =
        regex::Regex::new(r"class\s+(\w+)\s*\([^)]*View[^)]*\)").expect("django cbv regex");
    let re_fbv = regex::Regex::new(r"def\s+(\w+)\s*\(\s*request").expect("django fbv regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'python'")
        .context("prepare Python files for view scan")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
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
// PythonGrpcConnector removed — python resolver emits RpcCall during chain
// walking.

// ===========================================================================
// PythonGraphQlConnector — GraphQL resolver stops
// ===========================================================================

/// Detects Python GraphQL resolvers for Strawberry, Ariadne, and Graphene.
///
/// Start points come from .graphql schema files (graphql language plugin).
/// This connector emits Stop points for decorated resolvers and Graphene
/// `resolve_*` method conventions.
// PythonGraphQlConnector removed — python resolver emits GraphQLOp during
// chain walking. The Graphene `resolve_*` cross-file pattern (which this
// connector also did) becomes a Phase F migration when Python schema
// detection lands in extract_python_graphql or RouteDiscovery.

/// Per-file Python GraphQL scan: emit Consumer
/// `NamedChannel { kind: GraphQLOp, .. }` for ariadne
/// `@query.field("name")` / `@mutation.field(...)` registrations and
/// strawberry `@strawberry.field` / `@strawberry.mutation` decorators.
///
/// Graphene-style `resolve_*` methods are left to the legacy DB path
/// because they need to be detected from indexed symbols, not raw source.
pub fn extract_python_graphql(
    source: &str,
) -> Vec<(u32, crate::indexer::resolve::flow_emit::FlowEmission)> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let re_ariadne =
        regex::Regex::new(r#"@(?:query|mutation|subscription)\.field\s*\(\s*['"]([^'"]+)['"]"#)
            .expect("python ariadne field regex");
    let re_strawberry = regex::Regex::new(
        r#"@strawberry\.(?:field|mutation|query|subscription)\s*(?:\([^)]*\))?\s*$"#,
    )
    .expect("python strawberry field regex");

    if !source.contains("@strawberry") && !source.contains(".field(") {
        return Vec::new();
    }

    let mut out: Vec<(u32, FlowEmission)> = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    for (line_idx, line_text) in lines.iter().enumerate() {
        let line_no = (line_idx + 1) as u32;

        for cap in re_ariadne.captures_iter(line_text) {
            out.push((
                line_no,
                FlowEmission::NamedChannel {
                    kind: NamedChannelKind::GraphQLOp,
                    name: cap[1].to_string(),
                    role: ChannelRole::Consumer,
                    method: None,
                    streaming: None,
                },
            ));
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
                out.push((
                    line_no,
                    FlowEmission::NamedChannel {
                        kind: NamedChannelKind::GraphQLOp,
                        name,
                        role: ChannelRole::Consumer,
                        method: None,
                        streaming: None,
                    },
                ));
            }
        }
    }

    out
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
