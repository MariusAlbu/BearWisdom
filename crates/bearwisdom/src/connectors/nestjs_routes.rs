// =============================================================================
// connectors/nestjs_routes.rs  —  NestJS routes connector
//
// Detects HTTP route definitions in TypeScript NestJS controllers and writes
// them to the `routes` table.
//
// Detection strategy:
//
//   1. Query all TypeScript files from the `files` table.
//   2. For each file, scan for a class-level @Controller('prefix') annotation
//      to obtain the path prefix.
//   3. Scan method-level @Get, @Post, @Put, @Delete, @Patch annotations and
//      combine each with the class prefix.
//   4. Resolve `symbol_id` from `symbols` by matching the handler method name
//      against the same file.
//   5. Insert into the `routes` table via INSERT OR IGNORE.
//
// Detection is regex-based; NestJS decorator syntax is regular enough that
// a tree-sitter pass would add complexity without meaningful accuracy gain.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A NestJS HTTP route extracted from decorator annotations.
#[derive(Debug, Clone)]
pub struct NestRoute {
    /// `files.id` of the controller file.
    pub file_id: i64,
    /// `symbols.id` of the handler method, if indexed.
    pub symbol_id: Option<i64>,
    /// HTTP method, uppercased: "GET", "POST", "PUT", "DELETE", "PATCH".
    pub http_method: String,
    /// The combined route template (class prefix + method route).
    pub route_template: String,
    /// The TypeScript method name.
    pub handler_name: String,
    /// 1-based line number of the method decorator.
    pub line: u32,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract NestJS routes without writing to the DB — for use by `NestjsRouteConnector`.
pub(super) fn extract_nestjs_routes_pub(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<NestRoute>> {
    let regexes = Regexes::build();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'typescript'")
        .context("Failed to prepare TypeScript files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TypeScript files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TypeScript file rows")?;

    let mut routes: Vec<NestRoute> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        extract_routes_from_source(conn, &source, file_id, &rel_path, &regexes, &mut routes);
    }

    Ok(routes)
}

/// Scan all indexed TypeScript files for NestJS route decorators and insert
/// the discovered routes into the `routes` table.
///
/// Returns the number of routes written.
pub fn connect(conn: &Connection, project_root: &Path) -> Result<u32> {
    let regexes = Regexes::build();

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'typescript'")
        .context("Failed to prepare TypeScript files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TypeScript files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TypeScript file rows")?;

    let mut routes: Vec<NestRoute> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable TypeScript file");
                continue;
            }
        };

        extract_routes_from_source(conn, &source, file_id, &rel_path, &regexes, &mut routes);
    }

    debug!(count = routes.len(), "NestJS routes found");
    write_routes(conn, &routes)?;
    Ok(routes.len() as u32)
}

// ---------------------------------------------------------------------------
// Internal helpers — regex bundle
// ---------------------------------------------------------------------------

struct Regexes {
    /// @Controller('prefix') or @Controller("prefix") or @Controller()
    controller: Regex,
    /// @Get('route') / @Post('route') / @Put('route') / @Delete('route') / @Patch('route')
    method_decorator: Regex,
    /// TypeScript/JavaScript method declaration: optional async, name(
    method_name: Regex,
}

impl Regexes {
    fn build() -> Self {
        Self {
            controller: Regex::new(
                r#"@Controller\s*\(\s*(?:['"]([^'"]*)['"]\s*)?\)"#,
            )
            .expect("controller regex is valid"),

            method_decorator: Regex::new(
                r#"@(Get|Post|Put|Delete|Patch)\s*\(\s*(?:['"]([^'"]*)['"]\s*)?\)"#,
            )
            .expect("method decorator regex is valid"),

            method_name: Regex::new(
                r"(?:async\s+)?(\w+)\s*\(",
            )
            .expect("method name regex is valid"),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers — source extraction
// ---------------------------------------------------------------------------

/// Scan a single source file for NestJS route decorators and append any found
/// routes to `out`.
fn extract_routes_from_source(
    conn: &Connection,
    source: &str,
    file_id: i64,
    rel_path: &str,
    re: &Regexes,
    out: &mut Vec<NestRoute>,
) {
    let lines: Vec<&str> = source.lines().collect();

    // First pass: find the class-level @Controller prefix.
    let class_prefix = find_controller_prefix(&lines, re);

    // Second pass: collect method-level decorators.
    // A pending decorator is consumed when we encounter the next method
    // declaration.
    let mut pending: Option<(u32, String, String)> = None; // (ann_line, method, route)

    for (idx, line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;

        if let Some(cap) = re.method_decorator.captures(line) {
            let http_method = cap[1].to_uppercase();
            let suffix = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let route_template = join_paths(&class_prefix, suffix);
            pending = Some((line_no, http_method, route_template));
            continue;
        }

        if let Some((ann_line, http_method, route_template)) = pending.take() {
            // Skip blank lines and annotation lines between decorator and method.
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('@') {
                // Re-queue the pending route and keep scanning.
                pending = Some((ann_line, http_method, route_template));
                continue;
            }

            if let Some(fn_cap) = re.method_name.captures(line) {
                let handler_name = fn_cap[1].to_string();

                // Skip TypeScript keywords that match the pattern.
                if is_ts_keyword(&handler_name) {
                    pending = Some((ann_line, http_method, route_template));
                    continue;
                }

                let symbol_id = lookup_symbol(conn, &handler_name, rel_path);

                out.push(NestRoute {
                    file_id,
                    symbol_id,
                    http_method,
                    route_template,
                    handler_name,
                    line: ann_line,
                });
            }
        }
    }
}

/// Scan for the first @Controller(...) annotation and return the normalised
/// prefix.  Returns an empty string if the controller has no argument.
fn find_controller_prefix(lines: &[&str], re: &Regexes) -> String {
    for line in lines {
        if let Some(cap) = re.controller.captures(line) {
            return cap
                .get(1)
                .map(|m| normalise_prefix(m.as_str()))
                .unwrap_or_default();
        }
    }
    String::new()
}

/// Strip trailing slash from prefix and leading slash from suffix, then join.
fn join_paths(prefix: &str, suffix: &str) -> String {
    let p = prefix.trim_end_matches('/');
    let s = suffix.trim_start_matches('/');
    if p.is_empty() && s.is_empty() {
        "/".to_string()
    } else if p.is_empty() {
        format!("/{s}")
    } else if s.is_empty() {
        p.to_string()
    } else {
        format!("{p}/{s}")
    }
}

/// Ensure prefix starts with `/` and has no trailing slash.
fn normalise_prefix(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

/// Returns true if the name is a TypeScript keyword that can appear before `(`.
fn is_ts_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "while" | "for" | "switch" | "catch" | "function" | "return" | "new"
    )
}

/// Look up a method/function symbol by name in the given file.
fn lookup_symbol(conn: &Connection, name: &str, rel_path: &str) -> Option<i64> {
    conn.query_row(
        "SELECT s.id FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.name = ?1 AND f.path = ?2
           AND s.kind IN ('method', 'function')
         LIMIT 1",
        rusqlite::params![name, rel_path],
        |r| r.get(0),
    )
    .optional()
}

// ---------------------------------------------------------------------------
// Internal helpers — DB writes
// ---------------------------------------------------------------------------

fn write_routes(conn: &Connection, routes: &[NestRoute]) -> Result<()> {
    for route in routes {
        let result = conn.execute(
            "INSERT OR IGNORE INTO routes
                (file_id, symbol_id, http_method, route_template, line)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                route.file_id,
                route.symbol_id,
                route.http_method,
                route.route_template,
                route.line,
            ],
        );

        if let Err(e) = result {
            debug!(
                err = %e,
                route = %route.route_template,
                "Failed to insert NestJS route"
            );
        }
    }

    info!(count = routes.len(), "NestJS routes written to routes table");
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "nestjs_routes_tests.rs"]
mod tests;
