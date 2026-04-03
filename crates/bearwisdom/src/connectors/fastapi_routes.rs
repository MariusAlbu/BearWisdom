// =============================================================================
// connectors/fastapi_routes.rs  —  FastAPI / Starlette route connector
//
// Detection passes over indexed Python files:
//
//   1. Router prefix scan — collect `router = APIRouter(prefix="/...")` and
//      `app.include_router(router, prefix="/...")` declarations so that
//      decorator-level routes can be resolved to their full path.
//
//   2. Route decorator scan — detect `@app.get("/route")`, `@app.post(...)`,
//      `@router.get("/route")`, etc.  The function defined on the very next
//      non-blank/non-decorator line is treated as the handler and its
//      symbol_id is looked up in the DB.
//
//   3. Prefix resolution — combine the variable prefix (if any) with the
//      decorator-level route to produce `resolved_route`.
//
// All detection is regex-based.  FastAPI's decorator syntax is regular enough
// that full AST parsing would add cost without meaningful accuracy gain.
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

/// Matches decorator route definitions:
///   @app.get("/users")
///   @router.post("/users/{user_id}")
///   @some_var.delete("/items/{item_id}")
///
/// Captures: (1) variable name, (2) HTTP method, (3) route path
fn build_decorator_regex() -> Regex {
    Regex::new(
        r#"@(\w+)\.(get|post|put|delete|patch|head|options)\s*\(\s*['"]([^'"]+)['"]"#,
    )
    .expect("fastapi decorator regex is valid")
}

/// Matches `router = APIRouter(prefix="/users")` or
///         `router = APIRouter(prefix='/users')`.
///
/// Captures: (1) variable name, (2) prefix path
fn build_apirouter_regex() -> Regex {
    Regex::new(r#"(\w+)\s*=\s*APIRouter\s*\([^)]*prefix\s*=\s*['"]([^'"]*)['"]\s*[,)]"#)
        .expect("fastapi APIRouter regex is valid")
}

/// Matches `app.include_router(router, prefix="/api/v1")` or
///         `app.include_router(some_router, prefix='/api/v1')`.
///
/// Also handles the bare `app.include_router(router)` form (no prefix capture).
///
/// Captures: (1) router variable name, (2) optional prefix path
fn build_include_router_regex() -> Regex {
    Regex::new(
        r#"include_router\s*\(\s*(\w+)(?:[^)]*prefix\s*=\s*['"]([^'"]*)['"]\s*)?[,)]"#,
    )
    .expect("fastapi include_router regex is valid")
}

/// Matches the `def handler_name(` line that follows a route decorator.
///
/// Captures: (1) function name
fn build_handler_def_regex() -> Regex {
    Regex::new(r"^\s*(?:async\s+)?def\s+(\w+)\s*\(").expect("fastapi handler def regex is valid")
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

/// Normalise the HTTP method string to uppercase.
fn normalise_method(method: &str) -> String {
    method.to_uppercase()
}

/// Join a prefix and a path, ensuring exactly one `/` between them.
fn join_prefix(prefix: &str, path: &str) -> String {
    match (prefix.trim_end_matches('/'), path.trim_start_matches('/')) {
        ("", p) => format!("/{p}"),
        (pre, "") => pre.to_owned(),
        (pre, p) => format!("{pre}/{p}"),
    }
}

// ---------------------------------------------------------------------------
// Pass 1: collect router prefixes for all Python files
// ---------------------------------------------------------------------------

/// Public alias for use by `FastApiRouteConnector`.
pub(super) fn collect_prefixes_pub(
    source: &str,
    re_apirouter: &regex::Regex,
    re_include: &regex::Regex,
) -> std::collections::HashMap<String, String> {
    collect_prefixes(source, re_apirouter, re_include)
}

/// Build a map of `variable_name → effective_prefix` for a single file's
/// source text.
///
/// Two sources of prefix:
///   - `router = APIRouter(prefix="/users")` — declared in this file
///   - `app.include_router(router, prefix="/api/v1")` — mount override
///
/// When both are present the prefixes are concatenated.
fn collect_prefixes(
    source: &str,
    re_apirouter: &Regex,
    re_include: &Regex,
) -> HashMap<String, String> {
    let mut declared: HashMap<String, String> = HashMap::new();
    let mut mounted: HashMap<String, String> = HashMap::new();

    for line in source.lines() {
        // APIRouter(prefix=...) declaration
        if let Some(cap) = re_apirouter.captures(line) {
            let var_name = cap[1].to_owned();
            let prefix = cap[2].to_owned();
            declared.insert(var_name, prefix);
        }

        // include_router(router, prefix=...)
        if let Some(cap) = re_include.captures(line) {
            let var_name = cap[1].to_owned();
            let mount_prefix = cap.get(2).map(|m| m.as_str()).unwrap_or("").to_owned();
            if !mount_prefix.is_empty() {
                mounted.insert(var_name, mount_prefix);
            }
        }
    }

    // Merge: effective prefix = mount_prefix + declared_prefix
    let mut result: HashMap<String, String> = declared.clone();
    for (var, mount) in &mounted {
        let declared_part = declared.get(var).map(|s| s.as_str()).unwrap_or("");
        result.insert(var.clone(), join_prefix(mount, declared_part));
    }
    // Also carry any mount-only entries (router var with no APIRouter decl here)
    for (var, mount) in &mounted {
        result.entry(var.clone()).or_insert_with(|| mount.clone());
    }

    result
}

// ---------------------------------------------------------------------------
// Pass 2 & 3: detect decorators and insert routes
// ---------------------------------------------------------------------------

fn detect_fastapi_routes(conn: &Connection, project_root: &Path) -> Result<u32> {
    let re_decorator = build_decorator_regex();
    let re_apirouter = build_apirouter_regex();
    let re_include = build_include_router_regex();
    let re_handler = build_handler_def_regex();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'python'")
        .context("Failed to prepare Python file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Python files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Python file rows")?;

    let mut route_count: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Python file");
                continue;
            }
        };

        // Pass 1: collect prefix map for this file
        let prefixes = collect_prefixes(&source, &re_apirouter, &re_include);

        let lines: Vec<&str> = source.lines().collect();
        let mut i = 0usize;

        while i < lines.len() {
            let line_text = lines[i];
            let line_no = (i + 1) as u32;

            if let Some(cap) = re_decorator.captures(line_text) {
                let var_name = &cap[1];
                let http_method = normalise_method(&cap[2]);
                let route_path = &cap[3];

                // Look up prefix for this router variable
                let prefix = prefixes.get(var_name).map(|s| s.as_str()).unwrap_or("");
                let resolved = join_prefix(prefix, route_path);

                // Scan forward (skipping blank lines and additional decorators)
                // to find the `def handler_name(` line
                let mut handler_name: Option<String> = None;
                let mut j = i + 1;
                while j < lines.len() {
                    let next = lines[j].trim();
                    if next.is_empty() || next.starts_with('@') {
                        j += 1;
                        continue;
                    }
                    if let Some(hcap) = re_handler.captures(lines[j]) {
                        handler_name = Some(hcap[1].to_owned());
                    }
                    break;
                }

                // Resolve symbol_id from the handler function name
                let symbol_id: Option<i64> = handler_name.as_deref().and_then(|name| {
                    conn.query_row(
                        "SELECT id FROM symbols
                         WHERE file_id = ?1 AND name = ?2 AND kind IN ('function', 'method')
                         LIMIT 1",
                        rusqlite::params![file_id, name],
                        |r| r.get(0),
                    )
                    .optional()
                });

                debug!(
                    var = var_name,
                    method = %http_method,
                    route = route_path,
                    resolved = %resolved,
                    handler = ?handler_name,
                    "FastAPI route detected"
                );

                let result = conn.execute(
                    "INSERT OR IGNORE INTO routes
                       (file_id, symbol_id, http_method, route_template, resolved_route, line)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        file_id,
                        symbol_id,
                        http_method,
                        route_path,
                        resolved,
                        line_no
                    ],
                );

                match result {
                    Ok(n) if n > 0 => route_count += 1,
                    Ok(_) => {}
                    Err(e) => debug!(err = %e, "Failed to insert FastAPI route"),
                }
            }

            i += 1;
        }
    }

    Ok(route_count)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Detect FastAPI / Starlette route definitions in all indexed Python files
/// and write them to the `routes` table.
///
/// Returns the total number of route rows inserted.
pub fn connect(conn: &Connection, project_root: &Path) -> Result<u32> {
    let route_count =
        detect_fastapi_routes(conn, project_root).context("FastAPI route detection failed")?;
    info!(route_count, "FastAPI routes detected");
    Ok(route_count)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "fastapi_routes_tests.rs"]
mod tests;
