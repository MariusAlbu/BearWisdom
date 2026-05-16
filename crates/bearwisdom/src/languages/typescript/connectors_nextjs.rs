// =============================================================================
// languages/typescript/connectors_nextjs.rs — Next.js file-based routing scan
//
// `discover_nextjs_routes` in the connectors facade calls `_nextjs_routes_inner`
// to walk indexed TS/JS files and turn Pages Router (`/pages/api/**`) and App
// Router (`/app/api/**/route.ts`) filenames into `routes` rows. `[param]` and
// `[...slug]` dynamic segments are normalised to `{param}` / `{slug}`.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;

use super::connectors::insert_ts_route;

pub(super) fn _nextjs_routes_inner(
    conn: &Connection,
    project_root: &Path,
) -> Result<u32> {
        // Matches [param] and [...param] dynamic segments.
        let re_dynamic = Regex::new(r"\[\.\.\.(\w+)\]|\[(\w+)\]")
            .expect("nextjs dynamic segment regex");
        // Matches exported HTTP method handlers in App Router route files.
        let re_method = Regex::new(
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

        let mut inserted: u32 = 0;

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

                // Pages Router handlers export a default function — no method
                // constraint. Empty `http_method` denotes "any method".
                if insert_ts_route(conn, file_id, None, "", &route, 1) {
                    inserted += 1;
                }
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
                            if insert_ts_route(
                                conn,
                                file_id,
                                None,
                                &method,
                                &route,
                                (line_idx + 1) as u32,
                            ) {
                                inserted += 1;
                            }
                            found_any = true;
                        }
                    }

                    // No explicit exports found — treat the file as handling GET.
                    if !found_any && insert_ts_route(conn, file_id, None, "GET", &route, 1) {
                        inserted += 1;
                    }
                }
            }
        }

    Ok(inserted)
}

// ===========================================================================
// Next.js helpers
// ===========================================================================

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
fn nextjs_dynamic_segments(re: &Regex, s: &str) -> String {
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
