// =============================================================================
// connectors/angular_di.rs  —  Angular Dependency Injection connector
//
// Detects Angular DI patterns in TypeScript files and creates `flow_edges`
// of type `di_binding`.
//
// Two scan passes are performed:
//
//   Pass 1 — Injectable collection
//     Find classes decorated with @Injectable (with or without options).
//     Record the class name and its file_id.
//
//   Pass 2 — Constructor injection sites
//     For each TypeScript file, scan constructor parameter lists for
//     `private|public|protected name: TypeName` where TypeName was found
//     in Pass 1.  For each match, emit a flow_edge:
//       source = the injecting file/class (consumer)
//       target = the injectable service's file/symbol
//
// Pattern coverage:
//   @Injectable()
//   @Injectable({ providedIn: 'root' })
//   constructor(private fooService: FooService)
//   constructor(public bar: BarService, protected baz: BazService)
//   providers: [FooService]  — noted but not used for edges; the
//                              constructor injection approach is sufficient
//                              for DI binding discovery.
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// An `@Injectable` class discovered in a TypeScript file.
#[derive(Debug, Clone)]
struct InjectableService {
    /// `files.id` of the file that declares the class.
    file_id: i64,
    /// 1-based line of the class declaration.
    line: u32,
    /// Simple class name (e.g. `UserService`).
    name: String,
    /// `true` when `providedIn: 'root'` (or equivalent) was detected.
    provided_in_root: bool,
}

/// A constructor injection site found in a TypeScript file.
#[derive(Debug, Clone)]
struct InjectionSite {
    /// `files.id` of the consuming file.
    consumer_file_id: i64,
    /// 1-based line of the constructor parameter.
    line: u32,
    /// Name of the injected type.
    injected_type: String,
    /// Name of the enclosing class (best-effort).
    consumer_class: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Scan all indexed TypeScript files for Angular DI patterns and create
/// `flow_edges` of type `di_binding`.
///
/// Returns the number of edges inserted.
pub fn connect(conn: &Connection, project_root: &Path) -> Result<u32> {
    let re_injectable = build_injectable_regex();
    let re_class = build_class_regex();
    let re_constructor_param = build_constructor_param_regex();
    let re_provided_in = build_provided_in_regex();

    let files = query_typescript_files(conn)?;

    // Pass 1: collect all @Injectable classes.
    let mut injectables: HashMap<String, InjectableService> = HashMap::new();

    for (file_id, rel_path) in &files {
        let source = read_file(project_root, rel_path);
        let source = match source {
            Some(s) => s,
            None => continue,
        };

        collect_injectables(
            &source,
            *file_id,
            &re_injectable,
            &re_class,
            &re_provided_in,
            &mut injectables,
        );
    }

    debug!(count = injectables.len(), "Angular @Injectable classes found");

    if injectables.is_empty() {
        info!(created = 0, "Angular DI connector: no injectable services found");
        return Ok(0);
    }

    // Pass 2: collect constructor injection sites.
    let mut sites: Vec<InjectionSite> = Vec::new();

    for (file_id, rel_path) in &files {
        let source = read_file(project_root, rel_path);
        let source = match source {
            Some(s) => s,
            None => continue,
        };

        collect_injection_sites(
            &source,
            *file_id,
            &re_constructor_param,
            &re_class,
            &injectables,
            &mut sites,
        );
    }

    debug!(count = sites.len(), "Angular constructor injection sites found");

    // Pass 3: write flow_edges.
    let created = insert_flow_edges(conn, &sites, &injectables)?;

    info!(created, "Angular DI connector: flow_edges inserted");
    Ok(created)
}

// ---------------------------------------------------------------------------
// Regex constructors
// ---------------------------------------------------------------------------

/// Matches `@Injectable` decorator lines (with or without options).
fn build_injectable_regex() -> Regex {
    Regex::new(r"@Injectable\s*\(").expect("injectable regex is valid")
}

/// Matches a class declaration line.  Capture 1 = class name.
fn build_class_regex() -> Regex {
    Regex::new(r"\bclass\s+(\w+)").expect("class regex is valid")
}

/// Matches constructor injection parameters.
/// Capture 1 = visibility modifier, capture 2 = parameter name, capture 3 = type.
fn build_constructor_param_regex() -> Regex {
    Regex::new(r"\b(private|public|protected)\s+(?:readonly\s+)?(\w+)\s*:\s*(\w+)")
        .expect("constructor param regex is valid")
}

/// Matches `providedIn` key in the @Injectable options.
fn build_provided_in_regex() -> Regex {
    Regex::new(r#"providedIn\s*:\s*['"](\w+)['"]"#).expect("providedIn regex is valid")
}

// ---------------------------------------------------------------------------
// Pass 1 helpers
// ---------------------------------------------------------------------------

fn collect_injectables(
    source: &str,
    file_id: i64,
    re_injectable: &Regex,
    re_class: &Regex,
    re_provided_in: &Regex,
    out: &mut HashMap<String, InjectableService>,
) {
    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        let line_text = lines[i];

        if re_injectable.is_match(line_text) {
            // Collect the decorator span (may span several lines for the options object).
            let decorator_text = collect_decorator_span(&lines, i);
            let provided_in_root = is_provided_in_root(&decorator_text, re_provided_in);

            // The class declaration should appear within the next few lines.
            let class_line_idx = find_class_declaration(&lines, i);

            if let Some(cls_idx) = class_line_idx {
                if let Some(cap) = re_class.captures(lines[cls_idx]) {
                    let class_name = cap[1].to_string();
                    out.insert(
                        class_name.clone(),
                        InjectableService {
                            file_id,
                            line: (cls_idx + 1) as u32,
                            name: class_name,
                            provided_in_root,
                        },
                    );
                    // Advance past the class line to avoid re-matching.
                    i = cls_idx + 1;
                    continue;
                }
            }
        }

        i += 1;
    }
}

/// Collect lines starting at `start` until the closing `)` of the decorator
/// call.  Returns a single concatenated string for option parsing.
fn collect_decorator_span(lines: &[&str], start: usize) -> String {
    let mut buf = String::new();
    let mut depth = 0i32;
    let mut found_open = false;

    for line in lines.iter().skip(start).take(10) {
        buf.push_str(line);
        buf.push(' ');

        for ch in line.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    found_open = true;
                }
                ')' => depth -= 1,
                _ => {}
            }
        }

        if found_open && depth <= 0 {
            break;
        }
    }

    buf
}

/// Returns `true` if the decorator span contains `providedIn: 'root'`
/// (or `"root"`).
fn is_provided_in_root(decorator_text: &str, re_provided_in: &Regex) -> bool {
    re_provided_in
        .captures(decorator_text)
        .map(|cap| cap[1].eq_ignore_ascii_case("root"))
        .unwrap_or(false)
}

/// Search forward from `decorator_line` (inclusive) for the first line that
/// contains a `class` keyword.  Limit to 10 lines to avoid false positives.
fn find_class_declaration(lines: &[&str], decorator_line: usize) -> Option<usize> {
    for (offset, line) in lines.iter().enumerate().skip(decorator_line).take(10) {
        if line.contains("class ") {
            return Some(offset);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Pass 2 helpers
// ---------------------------------------------------------------------------

fn collect_injection_sites(
    source: &str,
    consumer_file_id: i64,
    re_param: &Regex,
    re_class: &Regex,
    injectables: &HashMap<String, InjectableService>,
    out: &mut Vec<InjectionSite>,
) {
    let mut current_class = String::new();
    let mut in_constructor = false;
    let mut constructor_depth = 0i32;

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // Track the enclosing class name.
        if let Some(cap) = re_class.captures(line_text) {
            current_class = cap[1].to_string();
        }

        // Detect the start of a constructor.
        if line_text.contains("constructor(") || line_text.contains("constructor (") {
            in_constructor = true;
            constructor_depth = 0;
        }

        if in_constructor {
            for ch in line_text.chars() {
                match ch {
                    '(' => constructor_depth += 1,
                    ')' => {
                        constructor_depth -= 1;
                        if constructor_depth <= 0 {
                            in_constructor = false;
                        }
                    }
                    _ => {}
                }
            }

            // Scan for `private|public|protected name: Type` in this line.
            for cap in re_param.captures_iter(line_text) {
                let injected_type = cap[3].to_string();

                if injectables.contains_key(&injected_type) {
                    out.push(InjectionSite {
                        consumer_file_id,
                        line: line_no,
                        injected_type,
                        consumer_class: current_class.clone(),
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Pass 3 — DB writes
// ---------------------------------------------------------------------------

fn insert_flow_edges(
    conn: &Connection,
    sites: &[InjectionSite],
    injectables: &HashMap<String, InjectableService>,
) -> Result<u32> {
    let mut created: u32 = 0;

    for site in sites {
        let service = match injectables.get(&site.injected_type) {
            Some(s) => s,
            None => continue,
        };

        // Deduplicate: skip if an identical di_binding already exists between
        // these two files at this source line for this target symbol.
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM flow_edges
                 WHERE source_file_id = ?1
                   AND source_line    = ?2
                   AND target_file_id = ?3
                   AND target_symbol  = ?4
                   AND edge_type      = 'di_binding'",
                rusqlite::params![
                    site.consumer_file_id,
                    site.line,
                    service.file_id,
                    service.name,
                ],
                |r| r.get(0),
            )
            .unwrap_or(0);

        if exists > 0 {
            continue;
        }

        let metadata = serde_json::json!({
            "consumer_class": site.consumer_class,
            "provided_in_root": service.provided_in_root,
            "source": "angular_di",
        })
        .to_string();

        conn.execute(
            "INSERT INTO flow_edges (
                source_file_id, source_line, source_symbol, source_language,
                target_file_id, target_line, target_symbol, target_language,
                edge_type, confidence, metadata
             ) VALUES (?1, ?2, ?3, 'typescript', ?4, ?5, ?6, 'typescript',
                        'di_binding', 0.85, ?7)",
            rusqlite::params![
                site.consumer_file_id,
                site.line,
                site.consumer_class,
                service.file_id,
                service.line,
                service.name,
                metadata,
            ],
        )
        .context("Failed to insert Angular DI flow_edge")?;

        created += 1;
    }

    Ok(created)
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

fn query_typescript_files(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'typescript'")
        .context("Failed to prepare TypeScript files query")?;

    let files = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TypeScript files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TypeScript file rows")?;

    Ok(files)
}

fn read_file(project_root: &Path, rel_path: &str) -> Option<String> {
    let abs_path = project_root.join(rel_path);
    match std::fs::read_to_string(&abs_path) {
        Ok(s) => Some(s),
        Err(e) => {
            debug!(path = %abs_path.display(), err = %e, "Skipping unreadable TypeScript file");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "angular_di_tests.rs"]
mod tests;
