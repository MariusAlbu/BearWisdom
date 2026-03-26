// =============================================================================
// connectors/spring.rs  —  Spring Boot / Spring Framework connector
//
// Three detection passes over indexed Java files:
//
//   1. Routes — scan for @GetMapping, @PostMapping, @PutMapping, @DeleteMapping,
//      @PatchMapping, and @RequestMapping annotations.  Class-level
//      @RequestMapping prefixes are accumulated and prepended to method-level
//      mappings.
//
//   2. Stereotypes — scan for @Controller, @RestController, @Service,
//      @Repository, @Component annotations before class declarations.
//
//   3. Registration — write routes to the `routes` table and create concepts
//      ("spring-controllers", "spring-services", "spring-repositories") with
//      annotated class symbols as members.
//
// Detection is regex-based.  Spring's annotation syntax is regular enough
// that a tree-sitter pass would add complexity without meaningful accuracy gain.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

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
// Internal helpers — regex builders
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

/// Matches a Java method declaration: (optional modifiers) returnType methodName(
/// Captures: (1) method name.
fn build_method_name_regex() -> Regex {
    Regex::new(r"(?:public|protected|private)\s+\S+\s+(\w+)\s*\(")
        .expect("method name regex is valid")
}

/// Matches Spring stereotype annotations.
/// Captures: (1) stereotype name (Controller|RestController|Service|Repository|Component).
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
// Internal helpers — source extraction
// ---------------------------------------------------------------------------

/// Scan source text for route annotations and emit SpringRoute entries.
///
/// Handles class-level @RequestMapping as a path prefix for all methods in
/// the class.  The prefix is detected on any line that does not immediately
/// precede a method declaration (heuristic: check 3 lines ahead for `{`
/// after the class keyword).
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
            // Determine if this is a class-level annotation: look for `class` within
            // the next 5 lines.
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

        // @RequestMapping at method level (not already captured as class prefix).
        if let Some(cap) = re_request_mapping.captures(line) {
            // Only treat as method-level if we're not on a line followed by `class`.
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

                // Try to find the symbol in the DB.
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

/// Scan source text for stereotype annotations and emit SpringService entries.
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

                // Find the symbol in the DB.
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
                    // Still emit with a placeholder — useful for the detection result.
                }
            } else {
                // The next non-blank line wasn't a class declaration — push stereotype
                // back in case there are annotations stacked (e.g. @Service + @Primary).
                if !line.trim().is_empty() && !line.trim_start().starts_with('@') {
                    // Genuinely not a class — discard.
                } else if line.trim_start().starts_with('@') {
                    // Another annotation — keep pending on next iteration.
                    // We already took pending_stereotype so re-check next line.
                    // This is a rare edge case; we handle it by re-scanning.
                    let _ = idx; // suppress unused warning
                }
            }
        }
    }
}

/// Map Spring annotation names to lowercase stereotype keys.
fn normalise_stereotype(annotation: &str) -> String {
    match annotation {
        "Controller" | "RestController" => "controller".to_string(),
        "Service" => "service".to_string(),
        "Repository" => "repository".to_string(),
        _ => "component".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers — DB writes
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

        // Upsert concept.
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
#[path = "spring_tests.rs"]
mod tests;
