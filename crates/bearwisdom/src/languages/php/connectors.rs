// =============================================================================
// languages/php/connectors.rs  —  Laravel route connector
//
// Detects HTTP route definitions in PHP Laravel route files and inserts rows
// into the `routes` table.
//
// Supported patterns
// ------------------
//   Route::get('/path', [Controller::class, 'method'])
//   Route::post('/path', [Controller::class, 'method'])
//   Route::put | patch | delete | options | any (same structure)
//   Route::get('/path', 'Controller@method')              (legacy string syntax)
//   Route::match(['get','post'], '/path', handler)
//   Route::resource('name', Controller::class)            (RESTful — 7 routes)
//   Route::apiResource('name', Controller::class)         (RESTful — 5 routes, no new/edit)
//   Route::prefix('api')->group(function () { ... })
//   Route::group(['prefix' => 'api'], function () { ... })
//   Route::middleware('auth')->group(function () { ... })
//
// Prefix tracking
// ---------------
// Prefix groups are tracked with a stack.  Opening a `->group(` or `group(`
// call pushes the active prefix onto the stack; a closing `}` at the
// outermost indentation pops it.  This is a line-scanning heuristic — it
// works reliably for the conventional Laravel style where each group closure
// spans multiple lines.
//
// Route::resource expansion
// -------------------------
//   resource:    GET /x          index
//                GET /x/create   create
//                POST /x         store
//                GET /x/{id}     show
//                GET /x/{id}/edit edit
//                PUT /x/{id}     update
//                DELETE /x/{id}  destroy
//
//   apiResource: same minus create and edit (5 routes total)
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// LaravelRouteConnector — LanguagePlugin entry point
// ===========================================================================

pub struct LaravelRouteConnector;

impl Connector for LaravelRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "laravel_routes",
            protocols: &[Protocol::Rest],
            languages: &["php"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.php_packages.iter().any(|p| p.contains("laravel"))
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let routes = extract_laravel_routes_pub(conn, project_root)
            .context("Laravel route detection failed")?;

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
                framework: "laravel".to_string(),
                metadata: None,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Regex builders
// ---------------------------------------------------------------------------

/// Matches explicit HTTP method calls:
///   Route::(get|post|put|patch|delete|options|any)('path', handler)
/// Captures: (1) http verb, (2) route path
fn build_explicit_route_regex() -> Regex {
    Regex::new(
        r#"Route::(get|post|put|patch|delete|options|any)\s*\(\s*['"]([^'"]+)['"]"#,
    )
    .expect("laravel explicit route regex is valid")
}

/// Matches Route::match(['get','post'], '/path', handler)
/// Captures: (1) raw methods list, (2) route path
fn build_match_route_regex() -> Regex {
    Regex::new(
        r#"Route::match\s*\(\s*\[([^\]]+)\]\s*,\s*['"]([^'"]+)['"]"#,
    )
    .expect("laravel match route regex is valid")
}

/// Matches Route::resource('name', Controller::class) or with ::class omitted.
/// Captures: (1) resource name
fn build_resource_regex() -> Regex {
    Regex::new(r#"Route::resource\s*\(\s*['"]([^'"]+)['"]"#)
        .expect("laravel resource regex is valid")
}

/// Matches Route::apiResource('name', Controller::class).
/// Captures: (1) resource name
fn build_api_resource_regex() -> Regex {
    Regex::new(r#"Route::apiResource\s*\(\s*['"]([^'"]+)['"]"#)
        .expect("laravel apiResource regex is valid")
}

/// Matches a prefix declaration on either a fluent chain or a group array:
///   Route::prefix('api')
///   ['prefix' => 'api/v1']
/// Captures: (1) prefix string
fn build_prefix_regex() -> Regex {
    Regex::new(r#"(?:Route::prefix\s*\(\s*|'prefix'\s*=>\s*)['"]([^'"]+)['"]"#)
        .expect("laravel prefix regex is valid")
}

/// Detects the opening of a group closure on the same line as the call:
///   ->group(function () {
///   Route::group([...], function () {
fn build_group_open_regex() -> Regex {
    Regex::new(r"->group\s*\(|Route::group\s*\(").expect("laravel group open regex is valid")
}

/// Matches a closing brace that ends a group closure.
fn build_group_close_regex() -> Regex {
    Regex::new(r"^\s*\}\s*(?:\)|;)?").expect("laravel group close regex is valid")
}

// ---------------------------------------------------------------------------
// Resource expansion helpers
// ---------------------------------------------------------------------------

/// The seven standard resourceful routes for `Route::resource`.
const RESOURCE_ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "",        "index"),
    ("GET", "/create", "create"),
    ("POST", "",       "store"),
    ("GET", "/{id}",   "show"),
    ("GET", "/{id}/edit", "edit"),
    ("PUT", "/{id}",   "update"),
    ("DELETE", "/{id}", "destroy"),
];

/// The five API resourceful routes for `Route::apiResource` (no create/edit).
const API_RESOURCE_ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "",        "index"),
    ("POST", "",       "store"),
    ("GET", "/{id}",   "show"),
    ("PUT", "/{id}",   "update"),
    ("DELETE", "/{id}", "destroy"),
];

/// Normalise a resource name into a URL path segment.
/// `users` → `/users`, `api/photos` → `/api/photos`
fn resource_base_path(name: &str, prefix: &str) -> String {
    let seg = name.trim_matches('/');
    if prefix.is_empty() {
        format!("/{seg}")
    } else {
        let p = prefix.trim_end_matches('/');
        format!("{p}/{seg}")
    }
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

/// Join a prefix stack into a single path string.
fn join_prefix_stack(stack: &[String]) -> String {
    let combined = stack
        .iter()
        .flat_map(|s| s.split('/'))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if combined.is_empty() {
        String::new()
    } else {
        format!("/{combined}")
    }
}

/// Build a resolved route by prepending the current prefix to a template.
fn resolve_route(prefix: &str, template: &str) -> String {
    let t = template.trim_start_matches('/');
    if prefix.is_empty() {
        format!("/{t}")
    } else {
        let p = prefix.trim_end_matches('/');
        format!("{p}/{t}")
    }
}

/// Insert one row into the `routes` table.
fn insert_route(
    conn: &Connection,
    file_id: i64,
    symbol_id: Option<i64>,
    http_method: &str,
    route_template: &str,
    resolved_route: &str,
    line: u32,
) -> bool {
    let result = conn.execute(
        "INSERT OR IGNORE INTO routes
           (file_id, symbol_id, http_method, route_template, resolved_route, line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            file_id,
            symbol_id,
            http_method,
            route_template,
            resolved_route,
            line
        ],
    );

    match result {
        Ok(n) if n > 0 => true,
        Ok(_) => false,
        Err(e) => {
            debug!(err = %e, route = route_template, "Failed to insert Laravel route");
            false
        }
    }
}

/// Look up a controller method symbol in the DB.
/// `handler_hint` is the raw handler expression from the route definition —
/// we extract a bare method name from it and search by name in PHP symbols.
fn resolve_symbol(conn: &Connection, handler_hint: &str) -> Option<i64> {
    // Extract "method" from "[Controller::class, 'method']" or "Controller@method".
    let method_name = if let Some(at_pos) = handler_hint.find('@') {
        handler_hint[at_pos + 1..].trim().to_string()
    } else if let Some(comma_pos) = handler_hint.rfind(',') {
        handler_hint[comma_pos + 1..]
            .trim()
            .trim_matches(|c: char| c == '\'' || c == '"' || c == ']' || c == ')')
            .to_string()
    } else {
        return None;
    };

    if method_name.is_empty() {
        return None;
    }

    conn.query_row(
        "SELECT s.id FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 AND f.language = 'php'
           AND s.kind IN ('function', 'method')
         LIMIT 1",
        rusqlite::params![method_name],
        |r| r.get(0),
    )
    .optional()
}

// ---------------------------------------------------------------------------
// Core scanning logic
// ---------------------------------------------------------------------------

/// Scan a single PHP file and insert all detected routes.
/// Returns the number of route rows inserted.
fn scan_file(
    conn: &Connection,
    file_id: i64,
    source: &str,
) -> u32 {
    let re_explicit = build_explicit_route_regex();
    let re_match = build_match_route_regex();
    let re_resource = build_resource_regex();
    let re_api_resource = build_api_resource_regex();
    let re_prefix = build_prefix_regex();
    let re_group_open = build_group_open_regex();
    let re_group_close = build_group_close_regex();

    // Prefix stack: each entry is one prefix segment pushed when a group opens.
    let mut prefix_stack: Vec<String> = Vec::new();
    // Track how many group-opens we have seen at each stack depth so we know
    // when to pop on a closing brace.  Each entry corresponds to one pending pop.
    let mut depth_stack: Vec<u32> = Vec::new();
    // Brace depth within the current group body.
    let mut brace_depth: u32 = 0;

    let mut inserted: u32 = 0;

    for (line_idx, line) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;
        let active_prefix = join_prefix_stack(&prefix_stack);

        // ------------------------------------------------------------------
        // Prefix / group detection
        // ------------------------------------------------------------------

        // Detect a prefix on this line (may be on same line as ->group() or
        // on the preceding line for fluent chains).
        let pending_prefix: Option<String> = re_prefix
            .captures(line)
            .map(|cap| cap[1].to_string());

        // Detect group opening on this line.
        let opens_group = re_group_open.is_match(line);

        if opens_group {
            let seg = pending_prefix.unwrap_or_default();
            prefix_stack.push(seg);
            depth_stack.push(brace_depth);
            // The opening `{` on this line increments brace_depth below via
            // the character scan; we reset the inner brace counter.
        }

        // Count net braces on this line (for group boundary detection).
        for ch in line.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                    }
                    // Check if this closes the innermost tracked group.
                    if let Some(&enter_depth) = depth_stack.last() {
                        if brace_depth == enter_depth {
                            prefix_stack.pop();
                            depth_stack.pop();
                        }
                    }
                }
                _ => {}
            }
        }

        // Skip pure group/prefix lines — no route data on them.
        if opens_group || re_group_close.is_match(line) {
            continue;
        }

        // ------------------------------------------------------------------
        // Explicit verb routes: Route::get/post/put/patch/delete/options/any
        // ------------------------------------------------------------------
        if let Some(cap) = re_explicit.captures(line) {
            let verb = cap[1].to_uppercase();
            let template = cap[2].to_string();
            let resolved = resolve_route(&active_prefix, &template);
            let symbol_id = resolve_symbol(conn, line);

            if insert_route(conn, file_id, symbol_id, &verb, &template, &resolved, line_no) {
                inserted += 1;
                debug!(verb = %verb, route = %resolved, line = line_no, "Laravel route inserted");
            }
            continue;
        }

        // ------------------------------------------------------------------
        // Route::match(['get', 'post'], '/path', handler)
        // ------------------------------------------------------------------
        if let Some(cap) = re_match.captures(line) {
            let methods_raw = &cap[1];
            let template = cap[2].to_string();
            let resolved = resolve_route(&active_prefix, &template);
            let symbol_id = resolve_symbol(conn, line);

            // Parse the method list: ['get', 'post'] etc.
            let methods: Vec<String> = methods_raw
                .split(',')
                .filter_map(|s| {
                    let trimmed = s.trim().trim_matches(|c: char| c == '\'' || c == '"');
                    if trimmed.is_empty() { None } else { Some(trimmed.to_uppercase()) }
                })
                .collect();

            for verb in &methods {
                if insert_route(conn, file_id, symbol_id, verb, &template, &resolved, line_no) {
                    inserted += 1;
                    debug!(verb = %verb, route = %resolved, line = line_no, "Laravel match route inserted");
                }
            }
            continue;
        }

        // ------------------------------------------------------------------
        // Route::resource — expands to 7 RESTful routes
        // ------------------------------------------------------------------
        if let Some(cap) = re_resource.captures(line) {
            let name = cap[1].to_string();
            let base = resource_base_path(&name, &active_prefix);

            for (verb, suffix, _action) in RESOURCE_ROUTES {
                let template = format!("{name}{suffix}");
                let resolved = format!("{base}{suffix}");
                if insert_route(conn, file_id, None, verb, &template, &resolved, line_no) {
                    inserted += 1;
                }
            }
            debug!(resource = %name, base = %base, "Laravel resource expanded");
            continue;
        }

        // ------------------------------------------------------------------
        // Route::apiResource — expands to 5 RESTful routes (no create/edit)
        // ------------------------------------------------------------------
        if let Some(cap) = re_api_resource.captures(line) {
            let name = cap[1].to_string();
            let base = resource_base_path(&name, &active_prefix);

            for (verb, suffix, _action) in API_RESOURCE_ROUTES {
                let template = format!("{name}{suffix}");
                let resolved = format!("{base}{suffix}");
                if insert_route(conn, file_id, None, verb, &template, &resolved, line_no) {
                    inserted += 1;
                }
            }
            debug!(resource = %name, base = %base, "Laravel apiResource expanded");
            continue;
        }
    }

    inserted
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// A route extracted from a Laravel file, ready for conversion to ConnectionPoint.
pub(crate) struct LaravelRoute {
    pub(crate) file_id: i64,
    pub(crate) symbol_id: Option<i64>,
    pub(crate) http_method: String,
    pub(crate) resolved_route: String,
    pub(crate) line: u32,
}

/// Extract Laravel routes without writing to the DB — for use by `LaravelRouteConnector`.
pub(crate) fn extract_laravel_routes_pub(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<LaravelRoute>> {
    let re_explicit = build_explicit_route_regex();
    let re_match_re = build_match_route_regex();
    let re_resource = build_resource_regex();
    let re_api_resource = build_api_resource_regex();
    let re_prefix_re = build_prefix_regex();
    let re_group_open = build_group_open_regex();
    let re_group_close = build_group_close_regex();

    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language = 'php'
               AND (path LIKE '%routes%' OR path LIKE '%Route%')",
        )
        .context("Failed to prepare Laravel route file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query PHP route files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect PHP route file rows")?;

    let mut result = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        extract_laravel_routes_from_source(
            conn,
            file_id,
            &source,
            &re_explicit,
            &re_match_re,
            &re_resource,
            &re_api_resource,
            &re_prefix_re,
            &re_group_open,
            &re_group_close,
            &mut result,
        );
    }

    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn extract_laravel_routes_from_source(
    conn: &Connection,
    file_id: i64,
    source: &str,
    re_explicit: &Regex,
    re_match_re: &Regex,
    re_resource: &Regex,
    re_api_resource: &Regex,
    re_prefix_re: &Regex,
    re_group_open: &Regex,
    re_group_close: &Regex,
    out: &mut Vec<LaravelRoute>,
) {
    let mut prefix_stack: Vec<String> = Vec::new();
    let mut depth_stack: Vec<u32> = Vec::new();
    let mut brace_depth: u32 = 0;

    for (line_idx, line) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;
        let active_prefix = join_prefix_stack(&prefix_stack);

        let pending_prefix: Option<String> = re_prefix_re
            .captures(line)
            .map(|cap| cap[1].to_string());

        let opens_group = re_group_open.is_match(line);

        if opens_group {
            let seg = pending_prefix.unwrap_or_default();
            prefix_stack.push(seg);
            depth_stack.push(brace_depth);
        }

        for ch in line.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => {
                    if brace_depth > 0 {
                        brace_depth -= 1;
                    }
                    if let Some(&enter_depth) = depth_stack.last() {
                        if brace_depth == enter_depth {
                            prefix_stack.pop();
                            depth_stack.pop();
                        }
                    }
                }
                _ => {}
            }
        }

        if opens_group || re_group_close.is_match(line) {
            continue;
        }

        if let Some(cap) = re_explicit.captures(line) {
            let verb = cap[1].to_uppercase();
            let template = cap[2].to_string();
            let resolved = resolve_route(&active_prefix, &template);
            let symbol_id = resolve_symbol(conn, line);
            out.push(LaravelRoute { file_id, symbol_id, http_method: verb, resolved_route: resolved, line: line_no });
            continue;
        }

        if let Some(cap) = re_match_re.captures(line) {
            let methods_raw = &cap[1];
            let template = cap[2].to_string();
            let resolved = resolve_route(&active_prefix, &template);
            let symbol_id = resolve_symbol(conn, line);
            let methods: Vec<String> = methods_raw
                .split(',')
                .filter_map(|s| {
                    let trimmed = s.trim().trim_matches(|c: char| c == '\'' || c == '"');
                    if trimmed.is_empty() { None } else { Some(trimmed.to_uppercase()) }
                })
                .collect();
            for verb in methods {
                out.push(LaravelRoute { file_id, symbol_id, http_method: verb, resolved_route: resolved.clone(), line: line_no });
            }
            continue;
        }

        if let Some(cap) = re_resource.captures(line) {
            let name = cap[1].to_string();
            let base = resource_base_path(&name, &active_prefix);
            for (verb, suffix, _action) in RESOURCE_ROUTES {
                let resolved = format!("{base}{suffix}");
                out.push(LaravelRoute { file_id, symbol_id: None, http_method: verb.to_string(), resolved_route: resolved, line: line_no });
            }
            continue;
        }

        if let Some(cap) = re_api_resource.captures(line) {
            let name = cap[1].to_string();
            let base = resource_base_path(&name, &active_prefix);
            for (verb, suffix, _action) in API_RESOURCE_ROUTES {
                let resolved = format!("{base}{suffix}");
                out.push(LaravelRoute { file_id, symbol_id: None, http_method: verb.to_string(), resolved_route: resolved, line: line_no });
            }
            continue;
        }
    }
}

/// Detect Laravel HTTP routes in all indexed PHP route files and write them
/// to the `routes` table.
///
/// Returns the total number of route rows inserted.
pub fn connect(conn: &Connection, project_root: &Path) -> Result<u32> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language = 'php'
               AND (path LIKE '%routes%' OR path LIKE '%Route%')",
        )
        .context("Failed to prepare Laravel route file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query PHP route files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect PHP route file rows")?;

    let mut total: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable PHP route file");
                continue;
            }
        };

        let count = scan_file(conn, file_id, &source);
        debug!(file = %rel_path, routes = count, "Laravel file scanned");
        total += count;
    }

    info!(total, "Laravel routes detected");
    Ok(total)
}

// ===========================================================================
// PhpRestConnector — HTTP client call starts + route stops for PHP
// ===========================================================================

pub struct PhpRestConnector;

impl Connector for PhpRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "php_rest",
            protocols: &[Protocol::Rest],
            languages: &["php"],
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
        extract_php_rest_stops(conn, &mut points)?;
        extract_php_rest_starts(conn, project_root, &mut points)?;
        Ok(points)
    }
}

fn extract_php_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'php'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare PHP REST stops query")?;

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
        .context("Failed to query PHP routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read PHP route row")?;
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

fn extract_php_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // Guzzle: $client->get('url'), $client->post('url'), Http::get('url'), etc.
    let re = regex::Regex::new(
        r#"(?:\$\w+|Http)\s*->\s*(?P<method>get|post|put|delete|patch|head)\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
    ).expect("php http client regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'php'")
        .context("Failed to prepare PHP files query")?;
    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query PHP files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect PHP file rows")?;

    for (file_id, rel_path) in files {
        if php_rest_is_test_file(&rel_path) {
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
                let raw_url = cap.name("url1")
                    .or_else(|| cap.name("url2"))
                    .map(|m| m.as_str().to_string());
                let Some(raw_url) = raw_url else { continue };
                if !php_rest_looks_like_api_url(&raw_url) {
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
    Ok(())
}

fn php_rest_is_test_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.contains("test") || lower.contains("spec")
}

fn php_rest_looks_like_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() { return false; }
        return php_rest_looks_like_api_url(path);
    }
    s.starts_with('/') || s.contains("/api/") || s.contains("/v1/") || s.contains("/v2/") || s.contains("/{")
}

fn rest_normalise_url_pattern(raw: &str) -> String {
    let without_query = raw.split('?').next().unwrap_or(raw);
    let re_tmpl = regex::Regex::new(r"\$\{[^}]+\}").expect("template regex");
    re_tmpl.replace_all(without_query, "{param}").into_owned()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
