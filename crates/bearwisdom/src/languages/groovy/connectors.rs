// =============================================================================
// languages/groovy/connectors.rs — Groovy Spring route discoverer
//
// Scans indexed Groovy files for Spring Web MVC route annotations
// (@GetMapping, @PostMapping, @PutMapping, @DeleteMapping, @PatchMapping,
// @RequestMapping) and writes them to the `routes` table. The
// routes-table → FlowEmission bridge in resolve/mod.rs emits Consumer flows.
// =============================================================================

use std::path::Path;

use regex::Regex;
use rusqlite::Connection;

use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// Public entry point
// ===========================================================================

/// Scan indexed Groovy files for Spring @*Mapping annotations and INSERT
/// rows into the `routes` table. Returns the count of newly-inserted routes.
pub fn discover_groovy_routes(
    conn: &Connection,
    project_root: &Path,
    _ctx: &ProjectContext,
) -> u32 {
    let mut stmt = match conn.prepare(
        "SELECT id, path FROM files WHERE language = 'groovy'",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("groovy routes: prepare files failed: {e}");
            return 0;
        }
    };

    let files: Vec<(i64, String)> = match stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .and_then(|it| it.collect::<rusqlite::Result<Vec<_>>>())
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("groovy routes: query files failed: {e}");
            return 0;
        }
    };

    let re_method = build_method_mapping_regex();
    let re_request = build_request_mapping_regex();
    let mut inserted: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !source.contains("Mapping") {
            continue;
        }

        let routes = scan_groovy_file(&source, &re_method, &re_request);

        if let Ok(mut ins) = conn.prepare_cached(
            "INSERT OR IGNORE INTO routes \
             (file_id, symbol_id, http_method, route_template, resolved_route, line) \
             VALUES (?1, NULL, ?2, ?3, ?3, ?4)",
        ) {
            for r in &routes {
                if let Ok(n) = ins.execute(rusqlite::params![
                    file_id,
                    r.http_method,
                    r.path,
                    r.line as i64,
                ]) {
                    if n > 0 {
                        inserted += 1;
                    }
                }
            }
        }
    }

    inserted
}

// ---------------------------------------------------------------------------
// Per-file scan
// ---------------------------------------------------------------------------

struct GroovyRoute {
    http_method: String,
    path: String,
    line: u32,
}

fn build_method_mapping_regex() -> Regex {
    Regex::new(
        r#"@(Get|Post|Put|Delete|Patch)Mapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["']"#,
    )
    .expect("groovy method mapping regex")
}

fn build_request_mapping_regex() -> Regex {
    Regex::new(
        r#"@RequestMapping\s*\(\s*(?:value\s*=\s*)?["']([^"']+)["'](?:[^)]*method\s*=\s*RequestMethod\.(\w+))?"#,
    )
    .expect("groovy request mapping regex")
}

fn scan_groovy_file(source: &str, re_method: &Regex, re_request: &Regex) -> Vec<GroovyRoute> {
    let lines: Vec<&str> = source.lines().collect();
    let mut class_prefix = String::new();
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        if let Some(cap) = re_request.captures(line) {
            let is_class_level = lines[idx + 1..]
                .iter()
                .take(3)
                .any(|l| l.trim_start().starts_with("class "));
            if is_class_level {
                class_prefix = cap[1].to_string();
                break;
            }
        }
    }

    for (idx, line_text) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;

        if let Some(cap) = re_method.captures(line_text) {
            let verb = cap[1].to_uppercase();
            let path = format!("{}{}", class_prefix, &cap[2]);
            out.push(GroovyRoute { http_method: verb, path, line: line_no });
            continue;
        }

        if let Some(cap) = re_request.captures(line_text) {
            let is_class_level = lines[idx + 1..]
                .iter()
                .take(3)
                .any(|l| l.trim_start().starts_with("class "));
            if !is_class_level {
                let verb = cap
                    .get(2)
                    .map(|m| m.as_str().to_uppercase())
                    .unwrap_or_else(|| "GET".to_string());
                let path = format!("{}{}", class_prefix, &cap[1]);
                out.push(GroovyRoute { http_method: verb, path, line: line_no });
            }
        }
    }

    out
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
