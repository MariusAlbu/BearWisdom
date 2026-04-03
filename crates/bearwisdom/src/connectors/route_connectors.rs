// =============================================================================
// connectors/route_connectors.rs — Framework-specific route producers
//
// Each connector wraps an existing route detection module and emits REST stop
// ConnectionPoints instead of writing to the `routes` table directly.
//
// The old connectors still run and populate `routes` for backward compat during
// migration.  These new connectors provide the same data through the registry
// pipeline, enabling the eventual removal of the legacy `routes` table path.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// Spring (Java)
// ===========================================================================

pub struct SpringRouteConnector;

impl Connector for SpringRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "spring_routes",
            protocols: &[Protocol::Rest],
            languages: &["java"],
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
        let routes = super::spring::find_spring_routes(conn, project_root)
            .context("Spring route detection failed")?;

        Ok(routes
            .into_iter()
            .map(|r| ConnectionPoint {
                file_id: r.file_id,
                symbol_id: r.symbol_id,
                line: r.line,
                protocol: Protocol::Rest,
                direction: FlowDirection::Stop,
                key: r.path,
                method: r.http_method,
                framework: "spring".to_string(),
                metadata: None,
            })
            .collect())
    }
}

// ===========================================================================
// Django (Python)
// ===========================================================================

pub struct DjangoRouteConnector;

impl Connector for DjangoRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "django_routes",
            protocols: &[Protocol::Rest],
            languages: &["python"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.python_packages.contains("django")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        // Django URLs: scan urls.py files for path()/re_path() calls and DRF router registrations.
        let re_url = regex::Regex::new(
            r#"(?:re_)?path\s*\(\s*r?['"]([^'"]+)['"]\s*,\s*(\w[\w.]*)"#,
        )
        .expect("django url regex");
        // DRF: router.register(r"prefix", ViewSetClass) or router.register("prefix", ViewSetClass)
        let re_router = regex::Regex::new(
            r#"\w+\.register\s*\(\s*r?['"]([^'"]+)['"]\s*,\s*(\w[\w.]*)"#,
        )
        .expect("drf router regex");

        let mut stmt = conn
            .prepare(
                "SELECT id, path FROM files
                 WHERE language = 'python' AND (path LIKE '%urls.py' OR path = 'urls.py')",
            )
            .context("Failed to prepare Django urls query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query Django url files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Django url files")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                // path() / re_path() patterns
                for cap in re_url.captures_iter(line_text) {
                    let route_path = cap[1].to_string();
                    let view_ref = &cap[2];
                    let view_name = view_ref.split('.').next_back().unwrap_or(view_ref);

                    let symbol_id: Option<i64> = conn
                        .query_row(
                            "SELECT s.id FROM symbols s
                             JOIN files f ON f.id = s.file_id
                             WHERE s.name = ?1 AND f.language = 'python'
                               AND s.kind IN ('function', 'class', 'method')
                             LIMIT 1",
                            rusqlite::params![view_name],
                            |r| r.get(0),
                        )
                        .ok();

                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Stop,
                        key: route_path,
                        method: "GET".to_string(), // Django routes handle all methods
                        framework: "django".to_string(),
                        metadata: None,
                    });
                }

                // DRF router.register(r"prefix", ViewSetClass)
                for cap in re_router.captures_iter(line_text) {
                    let prefix = format!("/{}", cap[1].trim_start_matches('/'));
                    let viewset = cap[2].to_string();

                    let symbol_id: Option<i64> = conn
                        .query_row(
                            "SELECT s.id FROM symbols s
                             JOIN files f ON f.id = s.file_id
                             WHERE s.name = ?1 AND f.language = 'python'
                               AND s.kind = 'class'
                             LIMIT 1",
                            rusqlite::params![viewset],
                            |r| r.get(0),
                        )
                        .ok();

                    // DRF routers generate list + detail routes; emit the prefix as a route.
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Stop,
                        key: prefix,
                        method: "GET".to_string(),
                        framework: "django".to_string(),
                        metadata: None,
                    });
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// FastAPI (Python)
// ===========================================================================

pub struct FastApiRouteConnector;

impl Connector for FastApiRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "fastapi_routes",
            protocols: &[Protocol::Rest],
            languages: &["python"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.python_packages.contains("fastapi") || ctx.python_packages.contains("starlette")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let re_decorator = regex::Regex::new(
            r#"@(\w+)\.(get|post|put|delete|patch|head|options)\s*\(\s*['"]([^'"]+)['"]"#,
        )
        .expect("fastapi decorator regex");
        let re_apirouter = regex::Regex::new(
            r#"(\w+)\s*=\s*APIRouter\s*\([^)]*prefix\s*=\s*['"]([^'"]*)['"]\s*[,)]"#,
        )
        .expect("fastapi APIRouter regex");
        let re_include = regex::Regex::new(
            r#"include_router\s*\(\s*(\w+)(?:[^)]*prefix\s*=\s*['"]([^'"]*)['"]\s*)?[,)]"#,
        )
        .expect("fastapi include_router regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'python'")
            .context("Failed to prepare Python files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query Python files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Python file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let prefixes =
                super::fastapi_routes::collect_prefixes_pub(&source, &re_apirouter, &re_include);

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                if let Some(cap) = re_decorator.captures(line_text) {
                    let var_name = &cap[1];
                    let http_method = cap[2].to_uppercase();
                    let route_path = &cap[3];

                    let prefix = prefixes.get(var_name).map(|s| s.as_str()).unwrap_or("");
                    let resolved = join_prefix(prefix, route_path);

                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Stop,
                        key: resolved,
                        method: http_method,
                        framework: "fastapi".to_string(),
                        metadata: None,
                    });
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// Go routes
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
        ctx.go_module_path.is_some()
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let routes = super::go_routes::extract_go_routes_pub(conn, project_root)
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

// ===========================================================================
// Rails (Ruby)
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

            let entries = super::rails_routes::parse_routes_source(&source);

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

// ===========================================================================
// Laravel (PHP)
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
        let routes = super::laravel_routes::extract_laravel_routes_pub(conn, project_root)
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

// ===========================================================================
// NestJS (TypeScript)
// ===========================================================================

pub struct NestjsRouteConnector;

impl Connector for NestjsRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "nestjs_routes",
            protocols: &[Protocol::Rest],
            languages: &["typescript"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.ts_packages.contains("@nestjs/core") || ctx.ts_packages.contains("@nestjs/common")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let routes = super::nestjs_routes::extract_nestjs_routes_pub(conn, project_root)
            .context("NestJS route detection failed")?;

        Ok(routes
            .into_iter()
            .map(|r| ConnectionPoint {
                file_id: r.file_id,
                symbol_id: r.symbol_id,
                line: r.line,
                protocol: Protocol::Rest,
                direction: FlowDirection::Stop,
                key: r.route_template,
                method: r.http_method,
                framework: "nestjs".to_string(),
                metadata: None,
            })
            .collect())
    }
}

// ===========================================================================
// Next.js (TypeScript / JavaScript)
// ===========================================================================

pub struct NextjsRouteConnector;

impl Connector for NextjsRouteConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "nextjs_routes",
            protocols: &[Protocol::Rest],
            languages: &["typescript", "tsx", "javascript", "jsx"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.ts_packages.contains("next")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        // Matches [param] and [...param] dynamic segments.
        let re_dynamic = regex::Regex::new(r"\[\.\.\.(\w+)\]|\[(\w+)\]")
            .expect("nextjs dynamic segment regex");
        // Matches exported HTTP method handlers in App Router route files.
        let re_method = regex::Regex::new(
            r"export\s+(?:async\s+)?function\s+(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS)\b",
        )
        .expect("nextjs route method regex");

        let mut stmt = conn
            .prepare(
                "SELECT id, path FROM files
                 WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
            )
            .context("Failed to prepare Next.js files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query Next.js files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Next.js file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            // Normalise separators for consistent matching on Windows.
            let norm = rel_path.replace('\\', "/");

            // ---- Pages Router: .../pages/api/**/*.{ts,tsx,js,jsx} ----
            if let Some(pos) = norm.find("/pages/api/") {
                let after_prefix = &norm[pos + "/pages/api/".len()..];
                let no_ext = nextjs_strip_ext(after_prefix);

                // Skip Next.js internals and middleware.
                let basename = no_ext.rsplit('/').next().unwrap_or(no_ext);
                if basename.starts_with('_') || basename == "middleware" {
                    continue;
                }

                let route_part = nextjs_dynamic_segments(&re_dynamic, no_ext);
                let route = if route_part == "index" || route_part.ends_with("/index") {
                    let base = route_part.trim_end_matches("/index").trim_end_matches("index");
                    if base.is_empty() {
                        "/api".to_string()
                    } else {
                        format!("/api/{}", base.trim_end_matches('/'))
                    }
                } else {
                    format!("/api/{route_part}")
                };

                // Pages Router handlers export a default function — no method constraint.
                points.push(ConnectionPoint {
                    file_id,
                    symbol_id: None,
                    line: 1,
                    protocol: Protocol::Rest,
                    direction: FlowDirection::Stop,
                    key: route,
                    method: String::new(), // matches any HTTP method
                    framework: "nextjs".to_string(),
                    metadata: None,
                });
                continue;
            }

            // ---- App Router: .../app/api/**/{route,page}.{ts,tsx,js,jsx} ----
            if norm.contains("/app/api/") {
                let basename_no_ext = nextjs_strip_ext(norm.rsplit('/').next().unwrap_or(""));
                if basename_no_ext != "route" {
                    continue;
                }

                if let Some(pos) = norm.find("/app/api/") {
                    // Everything between /app/api/ and the trailing /route.ext
                    let after_prefix = &norm[pos + "/app/api/".len()..];
                    let dir_part = after_prefix
                        .rsplit_once('/')
                        .map(|(dir, _)| dir)
                        .unwrap_or(""); // empty → route lives directly at /app/api/route.ts

                    let route = if dir_part.is_empty() {
                        "/api".to_string()
                    } else {
                        let tmpl = nextjs_dynamic_segments(&re_dynamic, dir_part);
                        format!("/api/{tmpl}")
                    };

                    // Read the file to find which HTTP methods are exported.
                    let abs_path = project_root.join(&rel_path);
                    let source = match std::fs::read_to_string(&abs_path) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };

                    let mut found_any = false;
                    for (line_idx, line_text) in source.lines().enumerate() {
                        if let Some(cap) = re_method.captures(line_text) {
                            let method = cap[1].to_string();
                            points.push(ConnectionPoint {
                                file_id,
                                symbol_id: None,
                                line: (line_idx + 1) as u32,
                                protocol: Protocol::Rest,
                                direction: FlowDirection::Stop,
                                key: route.clone(),
                                method,
                                framework: "nextjs".to_string(),
                                metadata: None,
                            });
                            found_any = true;
                        }
                    }

                    // No explicit exports found — treat the file as handling GET.
                    if !found_any {
                        points.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: 1,
                            protocol: Protocol::Rest,
                            direction: FlowDirection::Stop,
                            key: route,
                            method: "GET".to_string(),
                            framework: "nextjs".to_string(),
                            metadata: None,
                        });
                    }
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// Shared helpers
// ===========================================================================

fn join_prefix(prefix: &str, path: &str) -> String {
    match (prefix.trim_end_matches('/'), path.trim_start_matches('/')) {
        ("", p) => format!("/{p}"),
        (pre, "") => pre.to_owned(),
        (pre, p) => format!("{pre}/{p}"),
    }
}

/// Strip the file extension from a path fragment, leaving the rest intact.
/// Only strips the extension in the final path component.
fn nextjs_strip_ext(s: &str) -> &str {
    let last_slash = s.rfind('/').map(|i| i + 1).unwrap_or(0);
    let basename = &s[last_slash..];
    if let Some(dot) = basename.rfind('.') {
        &s[..last_slash + dot]
    } else {
        s
    }
}

/// Convert Next.js dynamic path segments to RFC 6570-style templates.
///
/// - `[...slug]` → `{slug}` (catch-all)
/// - `[param]`   → `{param}` (single dynamic segment)
fn nextjs_dynamic_segments(re: &regex::Regex, s: &str) -> String {
    re.replace_all(s, |caps: &regex::Captures| {
        let name = caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str())
            .unwrap_or("param");
        format!("{{{name}}}")
    })
    .to_string()
}
