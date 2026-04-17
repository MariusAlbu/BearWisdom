// =============================================================================
// languages/java/connectors.rs — Java language plugin connectors
//
// Spring Boot / Spring Framework route and DI connectors, moved here from
// connectors/route_connectors.rs + connectors/di_connector.rs.
//
// SpringRouteConnector:
//   Scans indexed Java files for @GetMapping/@PostMapping/etc. and
//   @RequestMapping annotations, emitting REST Stop connection points.
//
// SpringDiConnector:
//   Emits DI connection points (Start = interface, Stop = implementation)
//   by querying stereotype concept members and following `implements` edges.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// SpringRouteConnector
// ===========================================================================

pub struct SpringRouteConnector;

impl Connector for SpringRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "spring_routes",
            protocols: &[Protocol::Rest],
            languages: &["java"],
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
        let routes = find_spring_routes(conn, project_root)
            .context("Spring route detection failed")?;

        Ok(routes
            .into_iter()
            .map(|r| ConnectionPoint {
                file_id: r.file_id,
                symbol_id: r.symbol_id,
                line: r.line,
                protocol: Protocol::Rest,
                direction: FlowDirection::Stop,
                key: r.path,
                method: r.http_method,
                framework: "spring".to_string(),
                metadata: None,
            })
            .collect())
    }
}

// ===========================================================================
// SpringDiConnector
// ===========================================================================

pub struct SpringDiConnector;

impl Connector for SpringDiConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "spring_di",
            protocols: &[Protocol::Di],
            languages: &["java"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(
        &self,
        conn: &Connection,
        _project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        // Spring DI works from the existing symbol graph: stereotype classes
        // that have `implements` edges to interfaces get DI bindings.
        // Query stereotype members, then follow implements edges.
        let mut points = Vec::new();

        let mut stmt = conn
            .prepare(
                "SELECT cm.symbol_id, s.name, s.file_id, s.line
                 FROM concept_members cm
                 JOIN concepts c ON c.id = cm.concept_id
                 JOIN symbols s ON s.id = cm.symbol_id
                 WHERE c.name IN (
                     'spring-services',
                     'spring-repositories',
                     'spring-components'
                 )",
            )
            .context("Failed to query Spring stereotype members")?;

        let impls: Vec<(i64, String, i64, u32)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, u32>(3)?,
                ))
            })
            .context("Failed to execute Spring stereotype query")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Spring stereotype rows")?;

        for (impl_sym_id, impl_name, impl_file_id, impl_line) in &impls {
            // Find interfaces this class implements
            let mut iface_stmt = conn
                .prepare(
                    "SELECT tgt.name, tgt.file_id, tgt.line
                     FROM edges e
                     JOIN symbols tgt ON tgt.id = e.target_id
                     WHERE e.source_id = ?1
                       AND e.kind = 'implements'
                       AND tgt.kind = 'interface'",
                )
                .context("Failed to prepare implements query")?;

            let ifaces: Vec<(String, i64, u32)> = iface_stmt
                .query_map([impl_sym_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                })
                .context("Failed to query implements edges")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect implements rows")?;

            for (iface_name, iface_file_id, iface_line) in &ifaces {
                // Start: interface (the dependency being requested)
                points.push(ConnectionPoint {
                    file_id: *iface_file_id,
                    symbol_id: None,
                    line: *iface_line,
                    protocol: Protocol::Di,
                    direction: FlowDirection::Start,
                    key: iface_name.clone(),
                    method: String::new(),
                    framework: "spring".to_string(),
                    metadata: None,
                });

                // Stop: implementation (the type that fulfills the binding)
                points.push(ConnectionPoint {
                    file_id: *impl_file_id,
                    symbol_id: Some(*impl_sym_id),
                    line: *impl_line,
                    protocol: Protocol::Di,
                    direction: FlowDirection::Stop,
                    key: iface_name.clone(),
                    method: String::new(),
                    framework: "spring".to_string(),
                    metadata: Some(
                        serde_json::json!({
                            "implementation": impl_name,
                        })
                        .to_string(),
                    ),
                });
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// Spring route detection helpers (moved from connectors/spring.rs)
// ===========================================================================

/// A Spring route endpoint extracted from mapping annotations.
#[derive(Debug, Clone)]
pub struct SpringRoute {
    /// `files.id` of the controller file.
    pub file_id: i64,
    /// `symbols.id` of the handler method, if indexed.
    pub symbol_id: Option<i64>,
    /// HTTP method, uppercased: "GET", "POST", "PUT", "DELETE", "PATCH".
    pub http_method: String,
    /// The path pattern, including any class-level prefix.
    pub path: String,
    /// The Java method name.
    pub handler_name: String,
    /// 1-based line of the annotation.
    pub line: u32,
}

/// A Spring stereotype-annotated class.
#[derive(Debug, Clone)]
pub struct SpringService {
    /// `symbols.id` of the class.
    pub symbol_id: i64,
    /// Simple class name.
    pub name: String,
    /// One of "controller", "service", "repository", "component".
    pub stereotype: String,
}

/// Find Spring route annotations in all indexed Java files.
pub fn find_spring_routes(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<SpringRoute>> {
    let re_method_mapping = build_method_mapping_regex();
    let re_request_mapping = build_request_mapping_regex();
    let re_method_name = build_method_name_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'java'")
        .context("Failed to prepare Java files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Java files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Java file rows")?;

    let mut routes: Vec<SpringRoute> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Java file");
                continue;
            }
        };

        extract_routes_from_source(
            conn,
            &source,
            file_id,
            &rel_path,
            &re_method_mapping,
            &re_request_mapping,
            &re_method_name,
            &mut routes,
        );
    }

    debug!(count = routes.len(), "Spring routes found");
    Ok(routes)
}

/// Find Spring stereotype annotations in all indexed Java files.
pub fn find_spring_services(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<SpringService>> {
    let re_stereotype = build_stereotype_regex();
    let re_class = build_class_decl_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'java'")
        .context("Failed to prepare Java files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Java files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Java file rows")?;

    let mut services: Vec<SpringService> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Java file");
                continue;
            }
        };

        extract_services_from_source(
            conn,
            &source,
            file_id,
            &rel_path,
            &re_stereotype,
            &re_class,
            &mut services,
        );
    }

    debug!(count = services.len(), "Spring services found");
    Ok(services)
}

/// Write Spring routes to the `routes` table and create stereotype concepts.
pub fn register_spring_patterns(
    conn: &Connection,
    routes: &[SpringRoute],
    services: &[SpringService],
) -> Result<()> {
    write_routes(conn, routes)?;
    create_stereotype_concepts(conn, services)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Regex builders
// ---------------------------------------------------------------------------

/// Matches @GetMapping, @PostMapping, @PutMapping, @DeleteMapping, @PatchMapping.
/// Captures: (1) method verb, (2) path string.
fn build_method_mapping_regex() -> Regex {
    Regex::new(
        r#"@(Get|Post|Put|Delete|Patch)Mapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["']"#,
    )
    .expect("method mapping regex is valid")
}

/// Matches @RequestMapping with an optional method= argument.
/// Captures: (1) path string, (2) optional HTTP method (GET/POST/etc.).
fn build_request_mapping_regex() -> Regex {
    Regex::new(
        r#"@RequestMapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["'](?:[^)]*method\s*=\s*RequestMethod\.(\w+))?"#,
    )
    .expect("request mapping regex is valid")
}

/// Matches a Java method declaration.
/// Captures: (1) method name.
fn build_method_name_regex() -> Regex {
    Regex::new(r"(?:public|protected|private)\s+\S+\s+(\w+)\s*\(")
        .expect("method name regex is valid")
}

/// Matches Spring stereotype annotations.
/// Captures: (1) stereotype name.
fn build_stereotype_regex() -> Regex {
    Regex::new(r"@(Controller|RestController|Service|Repository|Component)\b")
        .expect("stereotype regex is valid")
}

/// Matches a Java class declaration.
/// Captures: (1) class name.
fn build_class_decl_regex() -> Regex {
    Regex::new(r"\bclass\s+(\w+)").expect("class decl regex is valid")
}

// ---------------------------------------------------------------------------
// Source extraction
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn extract_routes_from_source(
    conn: &Connection,
    source: &str,
    file_id: i64,
    rel_path: &str,
    re_method_mapping: &Regex,
    re_request_mapping: &Regex,
    re_method_name: &Regex,
    out: &mut Vec<SpringRoute>,
) {
    let lines: Vec<&str> = source.lines().collect();
    let mut class_prefix = String::new();

    // First pass: find class-level @RequestMapping for the prefix.
    for (idx, line) in lines.iter().enumerate() {
        if let Some(cap) = re_request_mapping.captures(line) {
            let is_class_level = lines
                .iter()
                .skip(idx + 1)
                .take(5)
                .any(|l| re_class_keyword(l));
            if is_class_level {
                class_prefix = normalise_path_prefix(&cap[1]);
            }
        }
    }

    // Second pass: collect method-level route annotations.
    let mut pending_route: Option<(u32, String, String)> = None; // (line, method, path)

    for (idx, line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;

        // @GetMapping / @PostMapping / etc.
        if let Some(cap) = re_method_mapping.captures(line) {
            let http_method = cap[1].to_uppercase();
            let path = join_paths(&class_prefix, &cap[2]);
            pending_route = Some((line_no, http_method, path));
            continue;
        }

        // @RequestMapping at method level.
        if let Some(cap) = re_request_mapping.captures(line) {
            let is_class_level = lines
                .iter()
                .skip(idx + 1)
                .take(5)
                .any(|l| re_class_keyword(l));
            if !is_class_level {
                let http_method = cap
                    .get(2)
                    .map(|m| m.as_str().to_uppercase())
                    .unwrap_or_else(|| "GET".to_string());
                let path = join_paths(&class_prefix, &cap[1]);
                pending_route = Some((line_no, http_method, path));
            }
            continue;
        }

        // Method declaration — consume the pending route.
        if let Some((ann_line, http_method, path)) = pending_route.take() {
            if let Some(fn_cap) = re_method_name.captures(line) {
                let handler_name = fn_cap[1].to_string();

                let symbol_id: Option<i64> = conn
                    .query_row(
                        "SELECT s.id FROM symbols s
                         JOIN files f ON f.id = s.file_id
                         WHERE s.name = ?1 AND f.path = ?2
                           AND s.kind IN ('method', 'function')
                         LIMIT 1",
                        rusqlite::params![handler_name, rel_path],
                        |r| r.get(0),
                    )
                    .optional();

                out.push(SpringRoute {
                    file_id,
                    symbol_id,
                    http_method,
                    path,
                    handler_name,
                    line: ann_line,
                });
            }
        }
    }
}

/// Quick heuristic: does `line` contain the keyword `class `?
fn re_class_keyword(line: &str) -> bool {
    line.contains(" class ") || line.starts_with("class ")
}

/// Strip trailing slash from prefix and leading slash from suffix, then join.
fn join_paths(prefix: &str, suffix: &str) -> String {
    let p = prefix.trim_end_matches('/');
    let s = suffix.trim_start_matches('/');
    if p.is_empty() {
        format!("/{s}")
    } else {
        format!("{p}/{s}")
    }
}

/// Normalise a class-level prefix: ensure it starts with `/`, strip trailing `/`.
fn normalise_path_prefix(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn extract_services_from_source(
    conn: &Connection,
    source: &str,
    _file_id: i64,
    rel_path: &str,
    re_stereotype: &Regex,
    re_class: &Regex,
    out: &mut Vec<SpringService>,
) {
    let lines: Vec<&str> = source.lines().collect();
    let mut pending_stereotype: Option<String> = None;

    for (idx, line) in lines.iter().enumerate() {
        if let Some(cap) = re_stereotype.captures(line) {
            let stereotype = normalise_stereotype(&cap[1]);
            pending_stereotype = Some(stereotype);
            continue;
        }

        if let Some(stereotype) = pending_stereotype.take() {
            if let Some(cap) = re_class.captures(line) {
                let class_name = cap[1].to_string();

                let symbol_id: Option<i64> = conn
                    .query_row(
                        "SELECT s.id FROM symbols s
                         JOIN files f ON f.id = s.file_id
                         WHERE s.name = ?1 AND f.path = ?2 AND s.kind = 'class'
                         LIMIT 1",
                        rusqlite::params![class_name, rel_path],
                        |r| r.get(0),
                    )
                    .optional();

                if let Some(sid) = symbol_id {
                    out.push(SpringService {
                        symbol_id: sid,
                        name: class_name,
                        stereotype,
                    });
                } else {
                    debug!(
                        class = %class_name,
                        "Spring stereotype class not found in symbol index"
                    );
                }
            } else if !line.trim().is_empty() && !line.trim_start().starts_with('@') {
                // Genuinely not a class — discard.
                let _ = idx;
            }
        }
    }
}

fn normalise_stereotype(annotation: &str) -> String {
    match annotation {
        "Controller" | "RestController" => "controller".to_string(),
        "Service" => "service".to_string(),
        "Repository" => "repository".to_string(),
        _ => "component".to_string(),
    }
}

// ---------------------------------------------------------------------------
// DB writes
// ---------------------------------------------------------------------------

fn write_routes(conn: &Connection, routes: &[SpringRoute]) -> Result<()> {
    for route in routes {
        let result = conn.execute(
            "INSERT OR IGNORE INTO routes
                (file_id, symbol_id, http_method, route_template, line)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                route.file_id,
                route.symbol_id,
                route.http_method,
                route.path,
                route.line,
            ],
        );

        match result {
            Ok(_) => {}
            Err(e) => {
                debug!(err = %e, path = %route.path, "Failed to insert Spring route");
            }
        }
    }

    info!(count = routes.len(), "Spring routes written to routes table");
    Ok(())
}

fn create_stereotype_concepts(conn: &Connection, services: &[SpringService]) -> Result<()> {
    let groups = [
        ("controller", "spring-controllers", "Spring @Controller / @RestController classes"),
        ("service", "spring-services", "Spring @Service classes"),
        ("repository", "spring-repositories", "Spring @Repository classes"),
        ("component", "spring-components", "Spring @Component classes"),
    ];

    for (stereotype, concept_name, description) in groups {
        let members: Vec<&SpringService> = services
            .iter()
            .filter(|s| s.stereotype == stereotype)
            .collect();

        if members.is_empty() {
            continue;
        }

        conn.execute(
            "INSERT OR IGNORE INTO concepts (name, description) VALUES (?1, ?2)",
            rusqlite::params![concept_name, description],
        )
        .context("Failed to upsert Spring concept")?;

        let concept_id: i64 = conn
            .query_row(
                "SELECT id FROM concepts WHERE name = ?1",
                [concept_name],
                |r| r.get(0),
            )
            .context("Failed to fetch concept id")?;

        for svc in &members {
            conn.execute(
                "INSERT OR IGNORE INTO concept_members (concept_id, symbol_id, auto_assigned)
                 VALUES (?1, ?2, 1)",
                rusqlite::params![concept_id, svc.symbol_id],
            )
            .context("Failed to insert Spring concept member")?;
        }

        info!(
            concept = concept_name,
            count = members.len(),
            "Spring concept populated"
        );
    }

    Ok(())
}

// ===========================================================================
// JavaRestConnector — HTTP client call starts + route stops for Java
// ===========================================================================

pub struct JavaRestConnector;

impl Connector for JavaRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "java_rest",
            protocols: &[Protocol::Rest],
            languages: &["java"],
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
        extract_java_rest_stops(conn, &mut points)?;
        extract_java_rest_starts(conn, project_root, &mut points)?;
        Ok(points)
    }
}

fn extract_java_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'java'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare Java REST stops query")?;

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
        .context("Failed to query Java routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) = row.context("Failed to read Java route row")?;
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

fn extract_java_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // HttpClient / RestTemplate / WebClient call sites
    let re = regex::Regex::new(
        r#"(?:HttpClient|RestTemplate|WebClient)[^.(]*\.\s*(?P<method>get|post|put|delete|getForObject|postForEntity|exchange|retrieve)\s*\([^)]*"(?P<url>[^"]+)""#,
    )
    .expect("java http client regex is valid");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'java'")
        .context("Failed to prepare Java files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Java files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Java file rows")?;

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
            for cap in re.captures_iter(line_text) {
                let Some(raw_url) = cap.name("url").map(|m| m.as_str().to_string()) else { continue };
                if !rest_looks_like_backend_api_url(&raw_url) {
                    continue;
                }
                let method = rest_normalise_method(cap.name("method").map(|m| m.as_str()).unwrap_or("GET"));
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
    Ok(())
}

// ---------------------------------------------------------------------------
// Extension trait for rusqlite::Connection
// ---------------------------------------------------------------------------

trait OptionalExt<T> {
    fn optional(self) -> Option<T>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared REST detection helpers (used by JavaRestConnector)
// ---------------------------------------------------------------------------

fn rest_is_test_or_config_file(rel_path: &str) -> bool {
    let lower_path = rel_path.to_lowercase();
    lower_path.contains("_test.") || lower_path.contains(".test.")
        || lower_path.contains("test/") || lower_path.contains("/tests/")
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

fn rest_normalise_method(raw: &str) -> String {
    match raw.to_lowercase().as_str() {
        "get" | "getforobject" | "getforlist" | "retrieve" => "GET".into(),
        "post" | "postforentity" | "postforobject" => "POST".into(),
        "put" => "PUT".into(),
        "delete" => "DELETE".into(),
        "patch" => "PATCH".into(),
        "head" => "HEAD".into(),
        "exchange" => "GET".into(),
        other => other.to_uppercase(),
    }
}

// ===========================================================================
// JavaGrpcConnector — gRPC service implementation stops
// ===========================================================================

/// Emits gRPC Stop connection points for Java gRPC service implementations.
///
/// Java gRPC stubs generate a base class `{ServiceName}Grpc.{ServiceName}ImplBase`.
/// Implementations override the RPC methods.  We find Java classes that extend
/// a class matching `*ImplBase` and emit stops for each method they declare.
pub struct JavaGrpcConnector;

impl Connector for JavaGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "java_grpc_stops",
            protocols: &[Protocol::Grpc],
            languages: &["java"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(&self, conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Find classes that extend a *ImplBase (gRPC generated stub base class).
        let mut stmt = conn
            .prepare(
                "SELECT s.name, s.file_id
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.language = 'java'
                   AND s.kind = 'class'
                   AND EXISTS (
                       SELECT 1 FROM edges e
                       JOIN symbols tgt ON tgt.id = e.target_id
                       WHERE e.source_id = s.id
                         AND e.kind = 'inherits'
                         AND tgt.name LIKE '%ImplBase'
                   )",
            )
            .context("Failed to prepare Java gRPC class query")?;

        let classes: Vec<(String, i64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .context("Failed to query Java gRPC classes")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Java gRPC class rows")?;

        let mut points = Vec::new();

        for (_class_name, file_id) in &classes {
            // Find the parent ImplBase name to derive the service name.
            let parent_name: Option<String> = conn
                .query_row(
                    "SELECT tgt.name FROM edges e
                     JOIN symbols src ON src.id = e.source_id
                     JOIN symbols tgt ON tgt.id = e.target_id
                     WHERE src.file_id = ?1
                       AND e.kind = 'inherits'
                       AND tgt.name LIKE '%ImplBase'
                     LIMIT 1",
                    rusqlite::params![file_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();

            // service_name = strip "ImplBase" suffix from "GreeterImplBase" → "Greeter"
            let service_name = parent_name
                .as_deref()
                .and_then(|n| n.strip_suffix("ImplBase"))
                .unwrap_or("")
                .to_string();

            if service_name.is_empty() {
                continue;
            }

            // Emit a stop for each method in this file.
            let mut method_stmt = conn
                .prepare(
                    "SELECT s.id, s.name, s.line
                     FROM symbols s
                     WHERE s.file_id = ?1 AND s.kind = 'method'",
                )
                .context("Failed to prepare Java gRPC method query")?;

            let methods: Vec<(i64, String, u32)> = method_stmt
                .query_map(rusqlite::params![file_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                })
                .context("Failed to query Java gRPC methods")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect Java gRPC method rows")?;

            for (sym_id, method_name, line) in methods {
                let key = format!("{service_name}.{method_name}");
                points.push(ConnectionPoint {
                    file_id: *file_id,
                    symbol_id: Some(sym_id),
                    line,
                    protocol: Protocol::Grpc,
                    direction: FlowDirection::Stop,
                    key,
                    method: String::new(),
                    framework: "grpc_java".to_string(),
                    metadata: None,
                });
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// JavaMqConnector — Message queue producer/consumer stops
// ===========================================================================

/// Detects Java message queue patterns: Spring Kafka, Spring AMQP (RabbitMQ),
/// AWS SQS (via Spring Cloud AWS), and raw Kafka client.
pub struct JavaMqConnector;

impl Connector for JavaMqConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "java_mq",
            protocols: &[Protocol::MessageQueue],
            languages: &["java"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.has_dependency(ManifestKind::Maven, "org.springframework.kafka")
            || ctx.has_dependency(ManifestKind::Gradle, "org.springframework.kafka")
            || ctx.has_dependency(ManifestKind::Maven, "org.springframework.amqp")
            || ctx.has_dependency(ManifestKind::Gradle, "org.springframework.amqp")
            || ctx.has_dependency(ManifestKind::Maven, "org.apache.kafka")
            || ctx.has_dependency(ManifestKind::Gradle, "org.apache.kafka")
            || ctx.has_dependency(ManifestKind::Maven, "software.amazon.awssdk")
            || ctx.has_dependency(ManifestKind::Gradle, "software.amazon.awssdk")
            || ctx.has_dependency(ManifestKind::Maven, "com.amazonaws")
            || ctx.has_dependency(ManifestKind::Gradle, "com.amazonaws")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Producers:
        //   kafkaTemplate.send("topic", ...)
        //   producer.send(new ProducerRecord<>("topic", ...))
        //   rabbitTemplate.convertAndSend("exchange", "routingKey", ...)
        //
        // Consumers:
        //   @KafkaListener(topics = "topic")
        //   @KafkaListener(topics = {"t1", "t2"})
        //   @RabbitListener(queues = "queue")

        let re_kafka_template_send = regex::Regex::new(
            r#"kafkaTemplate\.send\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("java kafka template send regex");

        let re_producer_record = regex::Regex::new(
            r#"new\s+ProducerRecord\s*<[^>]*>\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("java producer record regex");

        let re_rabbit_send = regex::Regex::new(
            r#"rabbitTemplate\.(?:convertAndSend|send)\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("java rabbit send regex");

        let re_kafka_listener = regex::Regex::new(
            r#"@KafkaListener\s*\([^)]*topics\s*=\s*(?:\{[^}]*['"]([^'"]+)['"]|['"]([^'"]+)['"])"#,
        )
        .expect("java kafka listener regex");

        let re_rabbit_listener = regex::Regex::new(
            r#"@RabbitListener\s*\([^)]*queues\s*=\s*(?:\{[^}]*['"]([^'"]+)['"]|['"]([^'"]+)['"])"#,
        )
        .expect("java rabbit listener regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'java'")
            .context("Failed to prepare Java files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to query Java files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Java file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                for cap in re_kafka_template_send.captures_iter(line_text) {
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

                for cap in re_producer_record.captures_iter(line_text) {
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

                for cap in re_rabbit_send.captures_iter(line_text) {
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

                for cap in re_kafka_listener.captures_iter(line_text) {
                    let topic = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str());
                    if let Some(t) = topic {
                        points.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::MessageQueue,
                            direction: FlowDirection::Stop,
                            key: t.to_string(),
                            method: String::new(),
                            framework: "kafka".to_string(),
                            metadata: None,
                        });
                    }
                }

                for cap in re_rabbit_listener.captures_iter(line_text) {
                    let queue = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str());
                    if let Some(q) = queue {
                        points.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::MessageQueue,
                            direction: FlowDirection::Stop,
                            key: q.to_string(),
                            method: String::new(),
                            framework: "rabbitmq".to_string(),
                            metadata: None,
                        });
                    }
                }
            }
        }

        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Tests (consolidated from connectors/spring_tests.rs and spring_di_tests.rs)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    // -----------------------------------------------------------------------
    // Unit tests for parsing helpers (from spring_tests.rs)
    // -----------------------------------------------------------------------

    #[test]
    fn method_mapping_regex_get_mapping() {
        let re = build_method_mapping_regex();
        let line = r#"    @GetMapping("/items")"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "Get");
        assert_eq!(&cap[2], "/items");
    }

    #[test]
    fn method_mapping_regex_post_mapping_value_form() {
        let re = build_method_mapping_regex();
        let line = r#"@PostMapping(value = "/orders")"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "Post");
        assert_eq!(&cap[2], "/orders");
    }

    #[test]
    fn method_mapping_regex_delete_mapping() {
        let re = build_method_mapping_regex();
        let line = r#"@DeleteMapping("/items/{id}")"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "Delete");
        assert_eq!(&cap[2], "/items/{id}");
    }

    #[test]
    fn request_mapping_regex_basic() {
        let re = build_request_mapping_regex();
        let line = r#"@RequestMapping("/api/catalog")"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "/api/catalog");
        assert!(cap.get(2).is_none());
    }

    #[test]
    fn request_mapping_regex_with_method() {
        let re = build_request_mapping_regex();
        let line = r#"@RequestMapping(value = "/orders", method = RequestMethod.POST)"#;
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "/orders");
        assert_eq!(&cap[2], "POST");
    }

    #[test]
    fn stereotype_regex_matches_controller() {
        let re = build_stereotype_regex();
        assert!(re.is_match("@RestController"));
        let cap = re.captures("@RestController").unwrap();
        assert_eq!(&cap[1], "RestController");
    }

    #[test]
    fn stereotype_regex_matches_service() {
        let re = build_stereotype_regex();
        let cap = re.captures("@Service").unwrap();
        assert_eq!(&cap[1], "Service");
    }

    #[test]
    fn normalise_stereotype_maps_rest_controller() {
        assert_eq!(normalise_stereotype("RestController"), "controller");
        assert_eq!(normalise_stereotype("Controller"), "controller");
    }

    #[test]
    fn normalise_stereotype_maps_service() {
        assert_eq!(normalise_stereotype("Service"), "service");
    }

    #[test]
    fn normalise_stereotype_maps_repository() {
        assert_eq!(normalise_stereotype("Repository"), "repository");
    }

    #[test]
    fn join_paths_combines_prefix_and_suffix() {
        assert_eq!(join_paths("/api", "/items"), "/api/items");
        assert_eq!(join_paths("/api/", "items"), "/api/items");
        assert_eq!(join_paths("", "/items"), "/items");
    }

    #[test]
    fn normalise_path_prefix_adds_leading_slash() {
        assert_eq!(normalise_path_prefix("api/catalog"), "/api/catalog");
        assert_eq!(normalise_path_prefix("/api/catalog/"), "/api/catalog");
    }

    // -----------------------------------------------------------------------
    // Source extraction tests (from spring_tests.rs)
    // -----------------------------------------------------------------------

    #[test]
    fn extracts_get_mapping_route() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('CatalogController.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();

        let source = r#"
@RestController
@RequestMapping("/api/catalog")
public class CatalogController {

    @GetMapping("/items")
    public List<Item> getItems() {
        return service.findAll();
    }
}
"#;

        let re_method = build_method_mapping_regex();
        let re_request = build_request_mapping_regex();
        let re_method_name = build_method_name_regex();
        let mut routes = Vec::new();

        extract_routes_from_source(
            conn,
            source,
            1,
            "CatalogController.java",
            &re_method,
            &re_request,
            &re_method_name,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "GET");
        assert_eq!(routes[0].path, "/api/catalog/items");
        assert_eq!(routes[0].handler_name, "getItems");
    }

    #[test]
    fn extracts_post_mapping_no_class_prefix() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('OrderController.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();

        let source = r#"
@RestController
public class OrderController {

    @PostMapping("/orders")
    public Order createOrder(@RequestBody OrderDto dto) {
        return orderService.create(dto);
    }
}
"#;

        let re_method = build_method_mapping_regex();
        let re_request = build_request_mapping_regex();
        let re_method_name = build_method_name_regex();
        let mut routes = Vec::new();

        extract_routes_from_source(
            conn,
            source,
            1,
            "OrderController.java",
            &re_method,
            &re_request,
            &re_method_name,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "POST");
        assert_eq!(routes[0].path, "/orders");
    }

    // -----------------------------------------------------------------------
    // Integration tests (from spring_tests.rs)
    // -----------------------------------------------------------------------

    fn seed_spring_db(db: &Database) -> (i64, i64) {
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/main/java/com/example/CatalogController.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'CatalogController', 'com.example.CatalogController', 'class', 5, 0)",
            [file_id],
        )
        .unwrap();
        let class_sym_id: i64 = conn.last_insert_rowid();

        (file_id, class_sym_id)
    }

    #[test]
    fn write_routes_inserts_to_routes_table() {
        let db = Database::open_in_memory().unwrap();
        let (file_id, _) = seed_spring_db(&db);

        let routes = vec![SpringRoute {
            file_id,
            symbol_id: None,
            http_method: "GET".to_string(),
            path: "/api/catalog/items".to_string(),
            handler_name: "getItems".to_string(),
            line: 10,
        }];

        write_routes(db.conn(), &routes).unwrap();

        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let (method, template): (String, String) = db
            .conn()
            .query_row(
                "SELECT http_method, route_template FROM routes",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(method, "GET");
        assert_eq!(template, "/api/catalog/items");
    }

    #[test]
    fn create_stereotype_concepts_creates_controller_concept() {
        let db = Database::open_in_memory().unwrap();
        let (_, class_sym_id) = seed_spring_db(&db);

        let services = vec![SpringService {
            symbol_id: class_sym_id,
            name: "CatalogController".to_string(),
            stereotype: "controller".to_string(),
        }];

        create_stereotype_concepts(db.conn(), &services).unwrap();

        let concept_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE name = 'spring-controllers'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(concept_count, 1);

        let member_count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM concept_members", [], |r| r.get(0))
            .unwrap();
        assert_eq!(member_count, 1);
    }

    #[test]
    fn create_stereotype_concepts_groups_by_type() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('Controller.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();
        let f1: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('Service.java', 'h2', 'java', 0)",
            [],
        )
        .unwrap();
        let f2: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'MyController', 'com.MyController', 'class', 1, 0)",
            [f1],
        )
        .unwrap();
        let ctrl_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'MyService', 'com.MyService', 'class', 1, 0)",
            [f2],
        )
        .unwrap();
        let svc_id: i64 = conn.last_insert_rowid();

        let services = vec![
            SpringService {
                symbol_id: ctrl_id,
                name: "MyController".to_string(),
                stereotype: "controller".to_string(),
            },
            SpringService {
                symbol_id: svc_id,
                name: "MyService".to_string(),
                stereotype: "service".to_string(),
            },
        ];

        create_stereotype_concepts(conn, &services).unwrap();

        let concept_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM concepts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(concept_count, 2, "Should have spring-controllers and spring-services");
    }

    #[test]
    fn register_spring_patterns_on_empty_inputs_is_noop() {
        let db = Database::open_in_memory().unwrap();
        register_spring_patterns(db.conn(), &[], &[]).unwrap();
    }

    // -----------------------------------------------------------------------
    // SpringDiConnector tests (from spring_di_tests.rs)
    // -----------------------------------------------------------------------

    fn seed_service_implements_interface(db: &Database) -> (i64, i64) {
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/IOrderService.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();
        let iface_file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/OrderService.java', 'h2', 'java', 0)",
            [],
        )
        .unwrap();
        let impl_file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'IOrderService', 'com.example.IOrderService', 'interface', 3, 0)",
            [iface_file_id],
        )
        .unwrap();
        let iface_symbol_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'OrderService', 'com.example.OrderService', 'class', 5, 0)",
            [impl_file_id],
        )
        .unwrap();
        let impl_symbol_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'implements', 5, 0.90)",
            [impl_symbol_id, iface_symbol_id],
        )
        .unwrap();

        (iface_symbol_id, impl_symbol_id)
    }

    fn register_spring_service_concept(db: &Database, impl_symbol_id: i64) {
        let conn = db.conn();

        conn.execute(
            "INSERT OR IGNORE INTO concepts (name, description)
             VALUES ('spring-services', 'Spring @Service classes')",
            [],
        )
        .unwrap();

        let concept_id: i64 = conn
            .query_row(
                "SELECT id FROM concepts WHERE name = 'spring-services'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        conn.execute(
            "INSERT OR IGNORE INTO concept_members (concept_id, symbol_id, auto_assigned)
             VALUES (?1, ?2, 1)",
            rusqlite::params![concept_id, impl_symbol_id],
        )
        .unwrap();
    }

    #[test]
    fn spring_di_connector_produces_connection_points() {
        let db = Database::open_in_memory().unwrap();
        let (_iface_id, impl_id) = seed_service_implements_interface(&db);
        register_spring_service_concept(&db, impl_id);

        let connector = SpringDiConnector;
        let points = connector
            .extract(db.conn(), std::path::Path::new("."))
            .unwrap();

        assert_eq!(points.len(), 2, "Expected one Start + one Stop");

        let start = points.iter().find(|p| p.direction == FlowDirection::Start).unwrap();
        let stop = points.iter().find(|p| p.direction == FlowDirection::Stop).unwrap();

        assert_eq!(start.key, "IOrderService");
        assert_eq!(stop.key, "IOrderService");
        assert_eq!(start.framework, "spring");
        assert_eq!(stop.framework, "spring");
    }

    #[test]
    fn spring_di_connector_empty_produces_no_points() {
        let db = Database::open_in_memory().unwrap();
        let connector = SpringDiConnector;
        let points = connector
            .extract(db.conn(), std::path::Path::new("."))
            .unwrap();
        assert!(points.is_empty());
    }

    #[test]
    fn spring_di_connector_service_without_implements_produces_no_points() {
        let db = Database::open_in_memory().unwrap();

        db.conn()
            .execute(
                "INSERT INTO files (path, hash, language, last_indexed)
                 VALUES ('src/StandaloneService.java', 'h1', 'java', 0)",
                [],
            )
            .unwrap();
        let file_id: i64 = db.conn().last_insert_rowid();

        db.conn()
            .execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
                 VALUES (?1, 'StandaloneService', 'com.example.StandaloneService', 'class', 3, 0)",
                [file_id],
            )
            .unwrap();
        let impl_id: i64 = db.conn().last_insert_rowid();

        register_spring_service_concept(&db, impl_id);

        let connector = SpringDiConnector;
        let points = connector
            .extract(db.conn(), std::path::Path::new("."))
            .unwrap();
        assert!(points.is_empty(), "No interface binding without implements edge");
    }
}
