// =============================================================================
// languages/go/connectors.rs  —  Go HTTP route connector
//
// Detects HTTP route registrations in Go source files and inserts them into
// the `routes` table.  Handles the following patterns:
//
//   stdlib / gorilla/mux
//     http.HandleFunc("/path", handler)
//     mux.HandleFunc("/path", handler)
//
//   Gin — r.GET("/path", handler), r.POST(...), etc.
//   Echo — e.GET("/path", handler), e.POST(...), etc.
//   Chi  — r.Get("/path", handler), r.Post(...), etc.
//
//   Generic HandleFunc with method constant
//     r.HandleFunc(http.MethodGet, "/path", handler)
//
//   Group prefix resolution
//     api := r.Group("/api")    →  varName -> "/api"
//     api.GET("/users", ...)    →  resolved = "/api/users"
//
// Detection is regex-based.  Go route registrations follow regular enough
// patterns that AST parsing adds cost without meaningful accuracy gain.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::types::{
    ConnectionKind, ConnectionPoint as AbstractPoint, ConnectionRole,
};

// ===========================================================================
// GoRouteConnector — LanguagePlugin entry point
// ===========================================================================

pub struct GoRouteConnector;

impl Connector for GoRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "go_routes",
            protocols: &[Protocol::Rest],
            languages: &["go"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.manifest(ManifestKind::GoMod).and_then(|m| m.module_path.as_ref()).is_some()
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let routes = extract_go_routes_pub(conn, project_root)
            .context("Go route detection failed")?;

        Ok(routes
            .into_iter()
            .map(|r| ConnectionPoint {
                file_id: r.file_id,
                symbol_id: r.symbol_id,
                line: r.line,
                protocol: Protocol::Rest,
                direction: FlowDirection::Stop,
                key: r.resolved_route,
                method: r.http_method,
                framework: "go".to_string(),
                metadata: None,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Regex builders
// ---------------------------------------------------------------------------

/// Matches stdlib / gorilla-mux:
///   http.HandleFunc("/path", handlerFn)
///   mux.HandleFunc("/path", handlerFn)
///   anyVar.HandleFunc("/path", handlerFn)
///
/// Capture groups: (1) path, (2) handler name.
fn build_handle_func_regex() -> Regex {
    Regex::new(r#"(?:\w+)\.HandleFunc\s*\(\s*"([^"]+)"\s*,\s*(\w+)"#)
        .expect("go HandleFunc regex is valid")
}

/// Matches Gin / Echo / similar capitalized method variants:
///   r.GET("/path", handler), r.POST(...), r.PUT(...), r.DELETE(...), r.PATCH(...)
///   e.GET("/path", handler), etc.
///
/// Capture groups: (1) receiver var, (2) HTTP method (uppercase), (3) path, (4) handler.
fn build_gin_style_regex() -> Regex {
    Regex::new(
        r#"(\w+)\.(GET|POST|PUT|DELETE|PATCH)\s*\(\s*"([^"]+)"\s*,\s*(\w+)"#,
    )
    .expect("go gin-style regex is valid")
}

/// Matches Chi / httprouter title-case variants:
///   r.Get("/path", handler), r.Post(...), r.Put(...), r.Delete(...), r.Patch(...)
///
/// Capture groups: (1) receiver var, (2) HTTP method (title-case), (3) path, (4) handler.
fn build_chi_style_regex() -> Regex {
    Regex::new(
        r#"(\w+)\.(Get|Post|Put|Delete|Patch)\s*\(\s*"([^"]+)"\s*,\s*(\w+)"#,
    )
    .expect("go chi-style regex is valid")
}

/// Matches generic HandleFunc with an explicit method constant as first arg:
///   r.HandleFunc(http.MethodGet, "/path", handler)
///
/// Capture groups: (1) receiver var, (2) method name (e.g. "MethodGet"), (3) path, (4) handler.
fn build_method_handle_func_regex() -> Regex {
    Regex::new(
        r#"(\w+)\.HandleFunc\s*\(\s*http\.(\w+)\s*,\s*"([^"]+)"\s*,\s*(\w+)"#,
    )
    .expect("go method HandleFunc regex is valid")
}

/// Matches group prefix declarations:
///   api := r.Group("/api")
///   v1 := router.Group("/v1")
///
/// Capture groups: (1) variable name, (2) prefix path.
fn build_group_regex() -> Regex {
    Regex::new(r#"(\w+)\s*:=\s*\w+\.Group\s*\(\s*"([^"]+)""#)
        .expect("go Group regex is valid")
}

// ---------------------------------------------------------------------------
// Internal helpers
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

/// Convert an `http.MethodXxx` constant name to an uppercase HTTP verb.
/// e.g. "MethodGet" → "GET", "MethodPost" → "POST".
/// Falls back to stripping "Method" prefix and uppercasing if unrecognised.
fn method_const_to_verb(constant: &str) -> String {
    match constant {
        "MethodGet" => "GET".to_string(),
        "MethodPost" => "POST".to_string(),
        "MethodPut" => "PUT".to_string(),
        "MethodDelete" => "DELETE".to_string(),
        "MethodPatch" => "PATCH".to_string(),
        "MethodHead" => "HEAD".to_string(),
        "MethodOptions" => "OPTIONS".to_string(),
        other => other
            .strip_prefix("Method")
            .unwrap_or(other)
            .to_uppercase(),
    }
}

/// Normalise a path prefix: ensure leading `/`, strip trailing `/`.
fn normalise_prefix(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

/// Join a (possibly empty) prefix with a route segment into a clean path.
fn join_paths(prefix: &str, segment: &str) -> String {
    let p = prefix.trim_end_matches('/');
    let s = segment.trim_start_matches('/');
    if p.is_empty() {
        format!("/{s}")
    } else {
        format!("{p}/{s}")
    }
}

// ---------------------------------------------------------------------------
// A single extracted route (before DB insert)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct GoRoute {
    pub(crate) file_id: i64,
    pub(crate) symbol_id: Option<i64>,
    pub(crate) http_method: String,
    pub(crate) route_template: String,
    pub(crate) resolved_route: String,
    pub(crate) handler_name: String,
    pub(crate) line: u32,
}

// ---------------------------------------------------------------------------
// Per-file extraction
// ---------------------------------------------------------------------------

struct Regexes {
    handle_func: Regex,
    gin: Regex,
    chi: Regex,
    method_handle_func: Regex,
    group: Regex,
}

fn extract_routes_from_source(
    source: &str,
    file_id: i64,
    conn: &Connection,
    rel_path: &str,
    re: &Regexes,
    out: &mut Vec<GoRoute>,
) {
    // First pass: collect group prefix bindings (varName → prefix).
    let mut groups: HashMap<String, String> = HashMap::new();
    for line in source.lines() {
        if let Some(cap) = re.group.captures(line) {
            groups.insert(cap[1].to_string(), normalise_prefix(&cap[2]));
        }
    }

    // Second pass: extract route registrations.
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // --- Generic method HandleFunc: r.HandleFunc(http.MethodGet, "/path", handler) ---
        if let Some(cap) = re.method_handle_func.captures(line_text) {
            let http_method = method_const_to_verb(&cap[2]);
            let raw_path = &cap[3];
            let handler_name = cap[4].to_string();
            let receiver = &cap[1];
            let prefix = groups.get(receiver).map(String::as_str).unwrap_or("");
            let resolved = join_paths(prefix, raw_path);

            let symbol_id = lookup_symbol(conn, &handler_name, rel_path);
            debug!(
                method = %http_method,
                route = %resolved,
                handler = %handler_name,
                line = line_no,
                "Go method HandleFunc route"
            );
            out.push(GoRoute {
                file_id,
                symbol_id,
                http_method,
                route_template: raw_path.to_string(),
                resolved_route: resolved,
                handler_name,
                line: line_no,
            });
            continue;
        }

        // --- stdlib / gorilla HandleFunc: xxx.HandleFunc("/path", handler) ---
        // (must come after method_handle_func to avoid partial overlap)
        if let Some(cap) = re.handle_func.captures(line_text) {
            let raw_path = &cap[1];
            let handler_name = cap[2].to_string();
            // HandleFunc without a method → treat as GET (stdlib convention)
            let http_method = "GET".to_string();
            let symbol_id = lookup_symbol(conn, &handler_name, rel_path);
            debug!(
                method = %http_method,
                route = %raw_path,
                handler = %handler_name,
                line = line_no,
                "Go HandleFunc route"
            );
            out.push(GoRoute {
                file_id,
                symbol_id,
                http_method,
                route_template: raw_path.to_string(),
                resolved_route: raw_path.to_string(),
                handler_name,
                line: line_no,
            });
            continue;
        }

        // --- Gin-style: r.GET("/path", handler) ---
        if let Some(cap) = re.gin.captures(line_text) {
            let receiver = &cap[1];
            let http_method = cap[2].to_string(); // already uppercase
            let raw_path = &cap[3];
            let handler_name = cap[4].to_string();
            let prefix = groups.get(receiver).map(String::as_str).unwrap_or("");
            let resolved = join_paths(prefix, raw_path);

            let symbol_id = lookup_symbol(conn, &handler_name, rel_path);
            debug!(
                method = %http_method,
                route = %resolved,
                handler = %handler_name,
                line = line_no,
                "Go gin-style route"
            );
            out.push(GoRoute {
                file_id,
                symbol_id,
                http_method,
                route_template: raw_path.to_string(),
                resolved_route: resolved,
                handler_name,
                line: line_no,
            });
            continue;
        }

        // --- Chi-style: r.Get("/path", handler) ---
        if let Some(cap) = re.chi.captures(line_text) {
            let receiver = &cap[1];
            let http_method = cap[2].to_uppercase(); // title-case → uppercase
            let raw_path = &cap[3];
            let handler_name = cap[4].to_string();
            let prefix = groups.get(receiver).map(String::as_str).unwrap_or("");
            let resolved = join_paths(prefix, raw_path);

            let symbol_id = lookup_symbol(conn, &handler_name, rel_path);
            debug!(
                method = %http_method,
                route = %resolved,
                handler = %handler_name,
                line = line_no,
                "Go chi-style route"
            );
            out.push(GoRoute {
                file_id,
                symbol_id,
                http_method,
                route_template: raw_path.to_string(),
                resolved_route: resolved,
                handler_name,
                line: line_no,
            });
            // no continue — chi regex is last
        }
    }
}

/// Look up a handler function symbol by name in the same file.
/// Falls back to a project-wide search across Go files if not found locally.
fn lookup_symbol(conn: &Connection, name: &str, rel_path: &str) -> Option<i64> {
    // Prefer a match in the same file.
    let local: Option<i64> = conn
        .query_row(
            "SELECT s.id FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.name = ?1 AND f.path = ?2
               AND s.kind IN ('function', 'method')
             LIMIT 1",
            rusqlite::params![name, rel_path],
            |r| r.get(0),
        )
        .optional();

    if local.is_some() {
        return local;
    }

    // Widen to all Go files.
    conn.query_row(
        "SELECT s.id FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 AND f.language = 'go'
           AND s.kind IN ('function', 'method')
         LIMIT 1",
        rusqlite::params![name],
        |r| r.get(0),
    )
    .optional()
}

// ---------------------------------------------------------------------------
// DB write
// ---------------------------------------------------------------------------

fn write_routes(conn: &Connection, routes: &[GoRoute]) -> Result<u32> {
    let mut inserted: u32 = 0;
    for route in routes {
        let result = conn.execute(
            "INSERT OR IGNORE INTO routes
               (file_id, symbol_id, http_method, route_template, resolved_route, line)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                route.file_id,
                route.symbol_id,
                route.http_method,
                route.route_template,
                route.resolved_route,
                route.line,
            ],
        );
        match result {
            Ok(n) if n > 0 => inserted += 1,
            Ok(_) => {}
            Err(e) => {
                debug!(
                    err = %e,
                    route = %route.route_template,
                    "Failed to insert Go route"
                );
            }
        }
    }
    Ok(inserted)
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Extract Go routes without writing to the DB — for use by `GoRouteConnector`.
pub(crate) fn extract_go_routes_pub(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<GoRoute>> {
    let re = Regexes {
        handle_func: build_handle_func_regex(),
        gin: build_gin_style_regex(),
        chi: build_chi_style_regex(),
        method_handle_func: build_method_handle_func_regex(),
        group: build_group_regex(),
    };

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'go'")
        .context("Failed to prepare Go files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Go files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Go file rows")?;

    let mut routes: Vec<GoRoute> = Vec::new();

    for (file_id, rel_path) in &files {
        let abs_path = project_root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        extract_routes_from_source(&source, *file_id, conn, rel_path, &re, &mut routes);
    }

    Ok(routes)
}

/// Scan all indexed Go files for HTTP route registrations and insert them
/// into the `routes` table.
///
/// Returns the number of routes inserted.
pub fn connect(conn: &Connection, project_root: &Path) -> Result<u32> {
    let re = Regexes {
        handle_func: build_handle_func_regex(),
        gin: build_gin_style_regex(),
        chi: build_chi_style_regex(),
        method_handle_func: build_method_handle_func_regex(),
        group: build_group_regex(),
    };

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'go'")
        .context("Failed to prepare Go files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Go files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Go file rows")?;

    let mut routes: Vec<GoRoute> = Vec::new();

    for (file_id, rel_path) in &files {
        let abs_path = project_root.join(rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Go file");
                continue;
            }
        };

        extract_routes_from_source(&source, *file_id, conn, rel_path, &re, &mut routes);
    }

    debug!(count = routes.len(), "Go routes found");

    let inserted = write_routes(conn, &routes)?;
    info!(inserted, "Go routes written to routes table");
    Ok(inserted)
}

// ===========================================================================
// GoRestConnector — HTTP client call starts + route stops for Go
// ===========================================================================

pub struct GoRestConnector;

impl Connector for GoRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "go_rest",
            protocols: &[Protocol::Rest],
            languages: &["go"],
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
        // Starts (http.Get, http.NewRequest) migrated into
        // `extract_go_rest_starts_src`. Stops stay on DB.
        let mut points = Vec::new();
        extract_go_rest_stops(conn, &mut points)?;
        Ok(points)
    }
}

fn extract_go_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'go'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare Go REST stops query")?;

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
        .context("Failed to query Go routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read Go route row")?;
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

fn go_rest_is_test_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.ends_with("_test.go") || lower.contains("/testdata/")
}

fn go_rest_looks_like_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() { return false; }
        return go_rest_looks_like_api_url(path);
    }
    s.starts_with('/') || s.contains("/api/") || s.contains("/v1/") || s.contains("/v2/") || s.contains("/v3/") || s.contains("/{")
}

fn rest_normalise_url_pattern(raw: &str) -> String {
    let without_query = raw.split('?').next().unwrap_or(raw);
    let re_tmpl = regex::Regex::new(r"\$\{[^}]+\}").expect("template regex");
    re_tmpl.replace_all(without_query, "{param}").into_owned()
}

// ===========================================================================
// GoGrpcConnector — gRPC service implementation stops
// ===========================================================================

/// Detects Go gRPC service implementations.
///
/// Go gRPC stubs generate an interface `{ServiceName}Server` and a registration
/// function `Register{ServiceName}Server`.  Concrete implementations satisfy
/// the interface.  We find structs that have methods registered via the generated
/// `Register*Server` pattern by looking at edges.
pub struct GoGrpcConnector;

impl Connector for GoGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "go_grpc_stops",
            protocols: &[Protocol::Grpc],
            languages: &["go"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        // google.golang.org/grpc is the standard Go gRPC module.
        ctx.has_dependency(ManifestKind::GoMod, "google.golang.org/grpc")
            || ctx.has_dependency(ManifestKind::GoMod, "google.golang.org")
    }

    fn extract(&self, conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Find structs that implement an interface named *Server (gRPC convention).
        let mut stmt = conn
            .prepare(
                "SELECT s.name, s.file_id
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.language = 'go'
                   AND s.kind = 'struct'
                   AND EXISTS (
                       SELECT 1 FROM edges e
                       JOIN symbols tgt ON tgt.id = e.target_id
                       WHERE e.source_id = s.id
                         AND e.kind = 'implements'
                         AND tgt.name LIKE '%Server'
                   )",
            )
            .context("Failed to prepare Go gRPC struct query")?;

        let structs: Vec<(String, i64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .context("Failed to query Go gRPC structs")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Go gRPC struct rows")?;

        let mut points = Vec::new();

        for (_struct_name, file_id) in &structs {
            let parent_name: Option<String> = conn
                .query_row(
                    "SELECT tgt.name FROM edges e
                     JOIN symbols src ON src.id = e.source_id
                     JOIN symbols tgt ON tgt.id = e.target_id
                     WHERE src.file_id = ?1
                       AND e.kind = 'implements'
                       AND tgt.name LIKE '%Server'
                     LIMIT 1",
                    rusqlite::params![file_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();

            // service_name = strip "Server" suffix → service name
            let service_name = parent_name
                .as_deref()
                .and_then(|n| n.strip_suffix("Server"))
                .unwrap_or("")
                .to_string();

            if service_name.is_empty() {
                continue;
            }

            // Emit stops for all methods in the same file.
            let mut method_stmt = conn
                .prepare(
                    "SELECT s.id, s.name, s.line
                     FROM symbols s
                     WHERE s.file_id = ?1 AND s.kind = 'method'",
                )
                .context("Failed to prepare Go gRPC method query")?;

            let methods: Vec<(i64, String, u32)> = method_stmt
                .query_map(rusqlite::params![file_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                })
                .context("Failed to query Go gRPC methods")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect Go gRPC method rows")?;

            for (sym_id, method_name, line) in methods {
                // Skip mustEmbedUnimplementedXxxServer — generated boilerplate.
                if method_name.starts_with("mustEmbed") || method_name.starts_with("Unimplemented") {
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
                    framework: "grpc_go".to_string(),
                    metadata: None,
                });
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// GoMqConnector — Message queue producer/consumer connection points
// ===========================================================================

/// Detects Go message queue patterns:
///   - sarama / confluent-kafka-go: `producer.SendMessage(...)` (producer)
///                                   `consumer.Messages()` (consumer)
///   - amqp091-go (RabbitMQ): `ch.Publish(exchange, routingKey, ...)` (producer)
///                              `ch.Consume(queue, ...)` (consumer)
///   - nats.go: `nc.Subscribe("subject", ...)` (consumer)
///              `nc.Publish("subject", ...)` (producer)
pub struct GoMqConnector;

impl Connector for GoMqConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "go_mq",
            protocols: &[Protocol::MessageQueue],
            languages: &["go"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.has_dependency(ManifestKind::GoMod, "github.com/Shopify/sarama")
            || ctx.has_dependency(ManifestKind::GoMod, "github.com/IBM/sarama")
            || ctx.has_dependency(ManifestKind::GoMod, "github.com/confluentinc/confluent-kafka-go")
            || ctx.has_dependency(ManifestKind::GoMod, "github.com/rabbitmq/amqp091-go")
            || ctx.has_dependency(ManifestKind::GoMod, "github.com/streadway/amqp")
            || ctx.has_dependency(ManifestKind::GoMod, "github.com/nats-io/nats.go")
    }

    fn extract(&self, _conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Flattened into `extract_go_mq_src`.
        Ok(Vec::new())
    }
}

// ===========================================================================
// Plugin-facing composer — called from GoPlugin::extract_connection_points
// ===========================================================================

pub fn extract_go_connection_points(source: &str, file_path: &str) -> Vec<AbstractPoint> {
    let mut out = Vec::new();
    extract_go_rest_starts_src(source, file_path, &mut out);
    extract_go_mq_src(source, &mut out);
    out
}

/// Go REST client-call starts: http.Get/Post + http.NewRequest.
pub fn extract_go_rest_starts_src(
    source: &str,
    file_path: &str,
    out: &mut Vec<AbstractPoint>,
) {
    if go_rest_is_test_file(file_path) {
        return;
    }
    if !source.contains("http.") {
        return;
    }

    let re_simple = regex::Regex::new(
        r#"http\s*\.\s*(?P<method>Get|Post)\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("go http.Get/Post regex");
    let re_new_request = regex::Regex::new(
        r#"http\s*\.\s*NewRequest\s*\(\s*"(?P<method>[^"]+)"\s*,\s*"(?P<url>[^"]+)""#,
    )
    .expect("go http.NewRequest regex");

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;
        for re in &[&re_simple, &re_new_request] {
            for cap in re.captures_iter(line_text) {
                let Some(raw_url) = cap.name("url").map(|m| m.as_str().to_string()) else {
                    continue;
                };
                if !go_rest_looks_like_api_url(&raw_url) {
                    continue;
                }
                let method = cap
                    .name("method")
                    .map(|m| m.as_str().to_uppercase())
                    .unwrap_or_else(|| "GET".to_string());
                let url_pattern = rest_normalise_url_pattern(&raw_url);
                let mut meta = HashMap::new();
                meta.insert("method".to_string(), method);
                out.push(AbstractPoint {
                    kind: ConnectionKind::Rest,
                    role: ConnectionRole::Start,
                    key: url_pattern,
                    line: line_no,
                    col: 1,
                    symbol_qname: String::new(),
                    meta,
                });
            }
        }
    }
}

/// Go MQ detection: sarama (topic field), amqp091 publish/consume, nats.
pub fn extract_go_mq_src(source: &str, out: &mut Vec<AbstractPoint>) {
    if !source.contains("Topic")
        && !source.contains(".Publish")
        && !source.contains(".Consume")
        && !source.contains(".Subscribe")
    {
        return;
    }

    let re_sarama_send =
        regex::Regex::new(r#"Topic\s*:\s*['"`]([^'"`]+)['"`]"#).expect("go sarama topic regex");
    let re_amqp_publish = regex::Regex::new(
        r#"\.Publish\s*\(\s*(?:ctx,\s*)?['"`]([^'"`]+)['"`]\s*,\s*['"`]([^'"`]+)['"`]"#,
    )
    .expect("go amqp publish regex");
    let re_amqp_consume =
        regex::Regex::new(r#"\.Consume\s*\(\s*['"`]([^'"`]+)['"`]"#).expect("go amqp consume regex");
    let re_nats_subscribe =
        regex::Regex::new(r#"nc\.Subscribe\s*\(\s*['"`]([^'"`]+)['"`]"#).expect("go nats sub regex");
    let re_nats_publish =
        regex::Regex::new(r#"nc\.Publish\s*\(\s*['"`]([^'"`]+)['"`]"#).expect("go nats pub regex");

    let push = |out: &mut Vec<AbstractPoint>,
                role: ConnectionRole,
                key: String,
                line: u32,
                framework: &str| {
        let mut meta = HashMap::new();
        meta.insert("framework".to_string(), framework.to_string());
        out.push(AbstractPoint {
            kind: ConnectionKind::MessageQueue,
            role,
            key,
            line,
            col: 1,
            symbol_qname: String::new(),
            meta,
        });
    };

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        for cap in re_sarama_send.captures_iter(line_text) {
            push(out, ConnectionRole::Start, cap[1].to_string(), line_no, "kafka");
        }
        for cap in re_amqp_publish.captures_iter(line_text) {
            push(out, ConnectionRole::Start, cap[2].to_string(), line_no, "rabbitmq");
        }
        for cap in re_amqp_consume.captures_iter(line_text) {
            push(out, ConnectionRole::Stop, cap[1].to_string(), line_no, "rabbitmq");
        }
        for cap in re_nats_subscribe.captures_iter(line_text) {
            push(out, ConnectionRole::Stop, cap[1].to_string(), line_no, "nats");
        }
        for cap in re_nats_publish.captures_iter(line_text) {
            push(out, ConnectionRole::Start, cap[1].to_string(), line_no, "nats");
        }
    }
}

#[cfg(test)]
mod plugin_source_scan_tests {
    use super::*;

    #[test]
    fn go_rest_http_get_is_start() {
        let mut out = Vec::new();
        extract_go_rest_starts_src(
            r#"resp, _ := http.Get("/api/ping")"#,
            "main.go",
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "/api/ping");
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("GET"));
    }

    #[test]
    fn go_rest_new_request_parses_method() {
        let mut out = Vec::new();
        extract_go_rest_starts_src(
            r#"req, _ := http.NewRequest("DELETE", "/api/users/42", nil)"#,
            "main.go",
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "/api/users/42");
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("DELETE"));
    }

    #[test]
    fn go_rest_skips_test_files() {
        let mut out = Vec::new();
        extract_go_rest_starts_src(
            r#"http.Get("/api/x")"#,
            "pkg/foo_test.go",
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn go_mq_sarama_producer_start() {
        let src = r#"msg := &sarama.ProducerMessage{Topic: "events", Value: v}"#;
        let mut out = Vec::new();
        extract_go_mq_src(src, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "events");
        assert_eq!(out[0].role, ConnectionRole::Start);
        assert_eq!(out[0].meta.get("framework").map(String::as_str), Some("kafka"));
    }

    #[test]
    fn go_mq_amqp_publish_uses_routing_key() {
        let src = r#"ch.Publish("exchange", "user.signup", false, false, msg)"#;
        let mut out = Vec::new();
        extract_go_mq_src(src, &mut out);
        let starts: Vec<_> = out.iter().filter(|p| p.role == ConnectionRole::Start).collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].key, "user.signup");
    }

    #[test]
    fn composer_combines_rest_and_mq() {
        let src = r#"
http.Get("/api/x")
ch.Publish("ex", "rk.a", false, false, m)
"#;
        let points = extract_go_connection_points(src, "main.go");
        let has = |k: ConnectionKind| points.iter().any(|p| p.kind == k);
        assert!(has(ConnectionKind::Rest));
        assert!(has(ConnectionKind::MessageQueue));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
