// =============================================================================
// languages/elixir/connectors.rs — Elixir-specific flow connectors
//
// PhoenixRouteConnector:
//   Scans indexed Elixir files for Phoenix Framework route definitions.
//
//   Phoenix routes are declared in a router module using macros:
//     get "/users", UserController, :index
//     post "/users", UserController, :create
//     resources "/users", UserController
//     scope "/api", MyAppWeb do ... end
//     pipe_through :browser
//
//   Detection: project has a mix.exs with phoenix dependency, OR the file
//   itself calls `use Phoenix.Router`.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::debug;

// ===========================================================================
// PhoenixRouteConnector
// ===========================================================================

/// Phoenix router map scan + `routes` table population. The
/// `routes`-table → FlowEmission bridge in resolve/mod.rs handles
/// downstream flow_edges emission.
///
/// Returns the count of routes written to the `routes` table.
pub fn discover_phoenix_routes(conn: &Connection, project_root: &Path) -> u32 {
    let routes = match find_phoenix_routes(conn, project_root) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Phoenix route detection failed: {e}");
            return 0;
        }
    };

    let mut inserted: u32 = 0;
    if let Ok(mut stmt) = conn.prepare_cached(
        "INSERT OR IGNORE INTO routes \
         (file_id, symbol_id, http_method, route_template, resolved_route, line) \
         VALUES (?1, NULL, ?2, ?3, ?3, ?4)",
    ) {
        for r in &routes {
            if let Ok(n) = stmt.execute(rusqlite::params![
                r.file_id,
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

    inserted
}

// ---------------------------------------------------------------------------
// Route record
// ---------------------------------------------------------------------------

struct PhoenixRoute {
    file_id: i64,
    http_method: String,
    path: String,
    line: u32,
}

// ---------------------------------------------------------------------------
// Regex builders
// ---------------------------------------------------------------------------

/// Explicit verb macros: get "/path", Controller, :action
fn build_verb_regex() -> Regex {
    Regex::new(
        r#"(?x)
        ^\s*
        (get|post|put|patch|delete|options|head)  # HTTP verb (group 1)
        \s+
        ["']([^"']+)["']                          # path (group 2)
        "#,
    )
    .expect("phoenix verb regex")
}

/// `resources "/path", Controller` — expands to standard CRUD routes.
fn build_resources_regex() -> Regex {
    Regex::new(r#"^\s*resources\s+["']([^"']+)["']"#).expect("phoenix resources regex")
}

/// `scope "/prefix" do` or `scope "/prefix", Module do`
fn build_scope_regex() -> Regex {
    Regex::new(r#"^\s*scope\s+["']([^"']+)["']"#).expect("phoenix scope regex")
}

/// `end` closing a block.
fn build_end_regex() -> Regex {
    Regex::new(r"^\s*end\s*$").expect("phoenix end regex")
}

/// `do` at end of line — opens a block.
fn build_do_regex() -> Regex {
    Regex::new(r"\bdo\s*$").expect("phoenix do regex")
}

// ---------------------------------------------------------------------------
// RESTful resource expansion
// ---------------------------------------------------------------------------

const RESOURCE_ROUTES: &[(&str, &str)] = &[
    ("GET", ""),
    ("GET", "/new"),
    ("POST", ""),
    ("GET", "/:id"),
    ("GET", "/:id/edit"),
    ("PUT", "/:id"),
    ("PATCH", "/:id"),
    ("DELETE", "/:id"),
];

fn expand_resources(base: &str, prefix: &str, line: u32) -> Vec<PhoenixRoute> {
    // file_id is set later in the caller — use 0 as placeholder; it won't be used.
    let _ = line;
    RESOURCE_ROUTES
        .iter()
        .map(|(method, suffix)| PhoenixRoute {
            file_id: 0,
            http_method: method.to_string(),
            path: format!("{}{}{}", prefix, base, suffix),
            line,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

fn find_phoenix_routes(conn: &Connection, project_root: &Path) -> Result<Vec<PhoenixRoute>> {
    let re_verb = build_verb_regex();
    let re_resources = build_resources_regex();
    let re_scope = build_scope_regex();
    let re_end = build_end_regex();
    let re_do = build_do_regex();

    // Only scan files that look like router modules.
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language = 'elixir'
               AND (path LIKE '%router%' OR path LIKE '%_router.ex%'
                    OR path LIKE '%/router.ex' OR path LIKE '%/router.exs')",
        )
        .context("Failed to prepare Elixir router file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .context("Failed to query Elixir router files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Elixir router file rows")?;

    let mut all_routes = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable Elixir file");
                continue;
            }
        };

        // Quick filter: must look like a Phoenix router. Modern Phoenix
        // apps put the `use Phoenix.Router` invocation behind a project
        // macro (`use MyAppWeb, :router`), so accept the project-macro
        // shape too. Generic `:router` plus any verb macro signature is
        // sufficient to gate the regex pass below.
        let looks_like_router = source.contains("Phoenix.Router")
            || source.contains("use Phoenix")
            || source.contains(", :router")
            || source.contains(":router\n");
        if !looks_like_router {
            continue;
        }

        let mut routes =
            parse_phoenix_routes(&source, &re_verb, &re_resources, &re_scope, &re_end, &re_do);

        // Back-fill file_id (expand_resources uses 0 as placeholder).
        for r in &mut routes {
            r.file_id = file_id;
        }

        all_routes.extend(routes);
    }

    debug!(count = all_routes.len(), "Phoenix routes found");
    Ok(all_routes)
}

fn parse_phoenix_routes(
    source: &str,
    re_verb: &Regex,
    re_resources: &Regex,
    re_scope: &Regex,
    re_end: &Regex,
    re_do: &Regex,
) -> Vec<PhoenixRoute> {
    // Stack of (prefix_segment, adds_prefix).
    let mut prefix_stack: Vec<(String, bool)> = Vec::new();
    let mut routes = Vec::new();

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // scope "/prefix" do ...
        if let Some(cap) = re_scope.captures(line_text) {
            let seg = cap[1].to_string();
            let seg = if seg.starts_with('/') {
                seg
            } else {
                format!("/{seg}")
            };
            prefix_stack.push((seg, true));
            continue;
        }

        // end
        if re_end.is_match(line_text) {
            prefix_stack.pop();
            continue;
        }

        // bare do block
        if re_do.is_match(line_text) && !re_scope.is_match(line_text) {
            prefix_stack.push((String::new(), false));
        }

        let current_prefix: String = prefix_stack
            .iter()
            .filter(|(_, adds)| *adds)
            .map(|(seg, _)| seg.as_str())
            .collect();

        // resources "/path", Controller
        if let Some(cap) = re_resources.captures(line_text) {
            let base = &cap[1];
            let expanded = expand_resources(base, &current_prefix, line_no);
            routes.extend(expanded);
            continue;
        }

        // get/post/put/patch/delete "/path", Controller, :action
        if let Some(cap) = re_verb.captures(line_text) {
            let verb = cap[1].to_uppercase();
            let path = format!("{}{}", current_prefix, &cap[2]);
            routes.push(PhoenixRoute {
                file_id: 0,
                http_method: verb,
                path,
                line: line_no,
            });
        }
    }

    routes
}
