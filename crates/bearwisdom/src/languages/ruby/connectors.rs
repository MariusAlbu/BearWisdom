// =============================================================================
// languages/ruby/connectors.rs  —  Rails routes connector
//
// Detects HTTP route definitions in Ruby on Rails `routes.rb` files and
// inserts them into the `routes` table.
//
// Supported patterns:
//   • Explicit verb methods:  get '/users', to: 'users#index'
//   • resources :name        — expanded to all 7 RESTful routes
//   • resources :name, only: [:index, :show]  — filtered expansion
//   • namespace :api do … end  — pushed onto the prefix stack
//   • scope '/path' do … end   — pushed onto the prefix stack
//   • root 'home#index'        — GET /
//
// Detection strategy:
//   1. Query files WHERE language = 'ruby' AND path matches a routes pattern.
//   2. Scan line-by-line, maintaining a prefix stack for namespace/scope blocks.
//   3. Insert into the `routes` table.
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
// RailsRouteConnector — LanguagePlugin entry point
// ===========================================================================

pub struct RailsRouteConnector;

impl Connector for RailsRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "rails_routes",
            protocols: &[Protocol::Rest],
            languages: &["ruby"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.ruby_gems.contains("rails") || ctx.ruby_gems.contains("railties")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut stmt = conn
            .prepare(
                "SELECT id, path FROM files
                 WHERE language = 'ruby'
                   AND (path LIKE '%routes.rb' OR path LIKE '%/routes/%')",
            )
            .context("Failed to prepare Rails route file query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query Ruby route files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Ruby route file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let entries = parse_routes_source(&source);

            for entry in entries {
                points.push(ConnectionPoint {
                    file_id,
                    symbol_id: None,
                    line: entry.line,
                    protocol: Protocol::Rest,
                    direction: FlowDirection::Stop,
                    key: entry.route_template,
                    method: entry.http_method.to_string(),
                    framework: "rails".to_string(),
                    metadata: None,
                });
            }
        }

        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Route record — intermediate representation before DB insert
// ---------------------------------------------------------------------------

pub(crate) struct RouteEntry {
    pub(crate) http_method: &'static str,
    pub(crate) route_template: String,
    pub(crate) line: u32,
}

// ---------------------------------------------------------------------------
// Regex builders
// ---------------------------------------------------------------------------

/// Matches explicit HTTP verb declarations:
///   get  '/users', to: 'users#index'
///   post "/users", to: "users#create"
///   put  '/users/:id', to: 'users#update'
///   (match/delete/patch handled identically)
fn build_verb_regex() -> Regex {
    Regex::new(
        r#"(?x)
        ^\s*
        (get|post|put|patch|delete|match)   # HTTP verb (group 1)
        \s+
        ['"]([^'"]+)['"]                    # route path (group 2)
        "#,
    )
    .expect("rails verb regex is valid")
}

/// Matches `root 'controller#action'` or `root to: 'controller#action'`.
fn build_root_regex() -> Regex {
    Regex::new(r"^\s*root\b").expect("rails root regex is valid")
}

/// Matches `resources :name` optionally followed by `, only: [...]`.
fn build_resources_regex() -> Regex {
    Regex::new(
        r#"(?x)
        ^\s*
        resources?\s+:(\w+)             # :resource_name (group 1)
        (?:
            .*?only\s*:\s*\[([^\]]*)\]  # only: [...] (group 2, optional)
        )?
        "#,
    )
    .expect("rails resources regex is valid")
}

/// Matches `namespace :name do` — prefix is /name.
fn build_namespace_regex() -> Regex {
    Regex::new(r"^\s*namespace\s+:(\w+)\s*\bdo\b").expect("rails namespace regex is valid")
}

/// Matches `scope '/path' do` or `scope path: '/path' do` — prefix is the literal path.
fn build_scope_regex() -> Regex {
    Regex::new(r#"^\s*scope\s+['"]([^'"]+)['"]\s*\bdo\b"#).expect("rails scope regex is valid")
}

/// Matches a bare `end` line (closes a block).
fn build_end_regex() -> Regex {
    Regex::new(r"^\s*end\s*$").expect("rails end regex is valid")
}

/// Matches `do` at the end of a line (opens a block we don't track prefix for,
/// but still need to count for `end` matching).
fn build_do_regex() -> Regex {
    Regex::new(r"\bdo\s*$").expect("rails do regex is valid")
}

// ---------------------------------------------------------------------------
// RESTful resource expansion
// ---------------------------------------------------------------------------

/// The 7 canonical Rails RESTful routes for `resources :name`.
const ALL_RESOURCE_ACTIONS: &[(&str, &str, &str)] = &[
    ("GET", "/{name}", "index"),
    ("GET", "/{name}/new", "new"),
    ("POST", "/{name}", "create"),
    ("GET", "/{name}/:id", "show"),
    ("GET", "/{name}/:id/edit", "edit"),
    ("PUT", "/{name}/:id", "update"),
    ("DELETE", "/{name}/:id", "destroy"),
];

/// Expand `resources :name` (with an optional `only:` filter) into route entries.
fn expand_resources(
    resource: &str,
    only_filter: Option<&str>,
    prefix: &str,
    line: u32,
) -> Vec<RouteEntry> {
    let allowed: Option<Vec<&str>> = only_filter.map(|s| {
        s.split(',')
            .map(|tok| tok.trim().trim_start_matches(':').trim())
            .collect()
    });

    ALL_RESOURCE_ACTIONS
        .iter()
        .filter(|(_, _, action)| {
            allowed
                .as_ref()
                .map(|list| list.contains(action))
                .unwrap_or(true)
        })
        .map(|(method, path_tmpl, _action)| {
            let path = path_tmpl.replace("{name}", resource);
            let full = format!("{}{}", prefix, path);
            RouteEntry {
                http_method: method,
                route_template: full,
                line,
            }
        })
        .collect()
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

/// Normalise an HTTP verb string to uppercase, mapping `match` → `GET`.
fn normalise_method(verb: &str) -> &'static str {
    match verb.to_ascii_lowercase().as_str() {
        "get" => "GET",
        "post" => "POST",
        "put" => "PUT",
        "patch" => "PATCH",
        "delete" => "DELETE",
        _ => "GET", // `match` and any unknown verbs
    }
}

// ---------------------------------------------------------------------------
// Core parsing logic — pure, no DB access
// ---------------------------------------------------------------------------

/// Parse a Rails routes file and return the list of detected routes.
///
/// `depth_stack` tracks (prefix, block_depth) for namespace/scope blocks.
/// Bare `do` blocks increment depth without adding a prefix; `end` pops the
/// most recently opened block.
pub(crate) fn parse_routes_source(source: &str) -> Vec<RouteEntry> {
    let re_verb = build_verb_regex();
    let re_root = build_root_regex();
    let re_resources = build_resources_regex();
    let re_namespace = build_namespace_regex();
    let re_scope = build_scope_regex();
    let re_end = build_end_regex();
    let re_do = build_do_regex();

    // Stack of (prefix_segment, opens_a_new_prefix).
    // `opens_a_new_prefix` distinguishes namespace/scope blocks (which add a
    // prefix) from bare `do` blocks (which only consume an `end`).
    let mut prefix_stack: Vec<(String, bool)> = Vec::new();
    let mut entries: Vec<RouteEntry> = Vec::new();

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // --- namespace :name do ------------------------------------------------
        if let Some(cap) = re_namespace.captures(line_text) {
            let seg = format!("/{}", &cap[1]);
            prefix_stack.push((seg, true));
            continue;
        }

        // --- scope '/path' do --------------------------------------------------
        if let Some(cap) = re_scope.captures(line_text) {
            let seg = cap[1].to_string();
            // Ensure the scope path starts with '/'.
            let seg = if seg.starts_with('/') {
                seg
            } else {
                format!("/{seg}")
            };
            prefix_stack.push((seg, true));
            continue;
        }

        // --- end ---------------------------------------------------------------
        if re_end.is_match(line_text) {
            prefix_stack.pop();
            continue;
        }

        // --- bare `do` block (resources ... do, etc.) --------------------------
        // Only push a non-prefix marker if the line opens a block but isn't
        // already handled above as namespace/scope.
        if re_do.is_match(line_text) && !re_namespace.is_match(line_text) && !re_scope.is_match(line_text) {
            prefix_stack.push((String::new(), false));
            // Fall through — the line may also contain route declarations.
        }

        // Build current prefix from stack entries that actually add segments.
        let current_prefix: String = prefix_stack
            .iter()
            .filter(|(_, adds)| *adds)
            .map(|(seg, _)| seg.as_str())
            .collect();

        // --- root --------------------------------------------------------------
        if re_root.is_match(line_text) {
            let full = format!("{}/", current_prefix);
            entries.push(RouteEntry {
                http_method: "GET",
                route_template: full,
                line: line_no,
            });
            continue;
        }

        // --- resources :name ---------------------------------------------------
        if let Some(cap) = re_resources.captures(line_text) {
            let resource = &cap[1];
            let only_filter = cap.get(2).map(|m| m.as_str());
            let mut expanded = expand_resources(resource, only_filter, &current_prefix, line_no);
            entries.append(&mut expanded);
            // Don't continue — if this line also has `do` we already pushed above.
            continue;
        }

        // --- explicit verb methods ---------------------------------------------
        if let Some(cap) = re_verb.captures(line_text) {
            let verb = normalise_method(&cap[1]);
            let path = &cap[2];
            let full = format!("{}{}", current_prefix, path);
            entries.push(RouteEntry {
                http_method: verb,
                route_template: full,
                line: line_no,
            });
        }
    }

    entries
}

// ---------------------------------------------------------------------------
// Database pass
// ---------------------------------------------------------------------------

fn detect_rails_routes(conn: &Connection, project_root: &Path) -> Result<u32> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language = 'ruby'
               AND (
                   path LIKE '%routes.rb'
                OR path LIKE '%/routes/%'
               )",
        )
        .context("Failed to prepare Rails routes file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Ruby route files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Ruby route file rows")?;

    let mut route_count: u32 = 0;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable routes.rb");
                continue;
            }
        };

        let entries = parse_routes_source(&source);

        for entry in entries {
            // Optionally find a matching Ruby symbol (controller action).
            let symbol_id: Option<i64> = conn
                .query_row(
                    "SELECT s.id FROM symbols s
                     JOIN files f ON f.id = s.file_id
                     WHERE f.language = 'ruby' AND s.kind IN ('method', 'function')
                     LIMIT 1",
                    rusqlite::params![],
                    |r| r.get(0),
                )
                .optional();

            debug!(
                method = entry.http_method,
                route = %entry.route_template,
                line = entry.line,
                "Rails route detected"
            );

            let result = conn.execute(
                "INSERT OR IGNORE INTO routes
                   (file_id, symbol_id, http_method, route_template, resolved_route, line)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                rusqlite::params![
                    file_id,
                    symbol_id,
                    entry.http_method,
                    entry.route_template,
                    entry.line,
                ],
            );

            match result {
                Ok(n) if n > 0 => route_count += 1,
                Ok(_) => {}
                Err(e) => debug!(err = %e, "Failed to insert Rails route"),
            }
        }
    }

    Ok(route_count)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Detect Rails route definitions and write them to the `routes` table.
///
/// Returns the total number of route rows inserted.
pub fn connect(conn: &Connection, project_root: &Path) -> Result<u32> {
    let route_count =
        detect_rails_routes(conn, project_root).context("Rails routes detection failed")?;
    info!(route_count, "Rails routes detected");
    Ok(route_count)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
