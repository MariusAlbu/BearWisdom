// =============================================================================
// connectors/go_routes.rs  —  Go HTTP route connector
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
struct GoRoute {
    file_id: i64,
    symbol_id: Option<i64>,
    http_method: String,
    route_template: String,
    resolved_route: String,
    handler_name: String,
    line: u32,
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
// Public entry point
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "go_routes_tests.rs"]
mod tests;
