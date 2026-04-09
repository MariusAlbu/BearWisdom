// =============================================================================
// languages/typescript/connectors.rs — TypeScript/NestJS/Next.js connectors
//
// Contains:
//   - NestjsRouteConnector  (@Controller / @Get / @Post … decorators)
//   - NextjsRouteConnector  (Pages Router /pages/api/ + App Router /app/api/)
//
// All NestJS route-extraction helpers are inlined here; there is no longer a
// dependency on the legacy `connectors/nestjs_routes.rs` module.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::debug;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// NestJS
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
        let routes = extract_nestjs_routes(conn, project_root)
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
// Next.js
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
// NestJS extraction helpers
// ===========================================================================

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

struct NestRegexes {
    /// @Controller('prefix') or @Controller("prefix") or @Controller()
    controller: Regex,
    /// @Get('route') / @Post('route') / @Put('route') / @Delete('route') / @Patch('route')
    method_decorator: Regex,
    /// TypeScript/JavaScript method declaration: optional async, name(
    method_name: Regex,
}

impl NestRegexes {
    fn build() -> Self {
        Self {
            controller: Regex::new(r#"@Controller\s*\(\s*(?:['"]([^'"]*)['"]\s*)?\)"#)
                .expect("controller regex is valid"),
            method_decorator: Regex::new(
                r#"@(Get|Post|Put|Delete|Patch)\s*\(\s*(?:['"]([^'"]*)['"]\s*)?\)"#,
            )
            .expect("method decorator regex is valid"),
            method_name: Regex::new(r"(?:async\s+)?(\w+)\s*\(")
                .expect("method name regex is valid"),
        }
    }
}

fn extract_nestjs_routes(conn: &Connection, project_root: &Path) -> Result<Vec<NestRoute>> {
    let re = NestRegexes::build();

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
        extract_routes_from_source(conn, &source, file_id, &rel_path, &re, &mut routes);
    }

    Ok(routes)
}

/// Scan a single source file for NestJS route decorators and append found routes to `out`.
fn extract_routes_from_source(
    conn: &Connection,
    source: &str,
    file_id: i64,
    rel_path: &str,
    re: &NestRegexes,
    out: &mut Vec<NestRoute>,
) {
    let lines: Vec<&str> = source.lines().collect();

    // First pass: find the class-level @Controller prefix.
    let class_prefix = find_controller_prefix(&lines, re);

    // Second pass: collect method-level decorators.
    let mut pending: Option<(u32, String, String)> = None; // (ann_line, method, route)

    for (idx, line) in lines.iter().enumerate() {
        let line_no = (idx + 1) as u32;

        if let Some(cap) = re.method_decorator.captures(line) {
            let http_method = cap[1].to_uppercase();
            let suffix = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let route_template = nest_join_paths(&class_prefix, suffix);
            pending = Some((line_no, http_method, route_template));
            continue;
        }

        if let Some((ann_line, http_method, route_template)) = pending.take() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('@') {
                pending = Some((ann_line, http_method, route_template));
                continue;
            }

            if let Some(fn_cap) = re.method_name.captures(line) {
                let handler_name = fn_cap[1].to_string();

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

fn find_controller_prefix(lines: &[&str], re: &NestRegexes) -> String {
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

fn nest_join_paths(prefix: &str, suffix: &str) -> String {
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

fn is_ts_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "while" | "for" | "switch" | "catch" | "function" | "return" | "new"
    )
}

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

// ===========================================================================
// rusqlite optional helper
// ===========================================================================

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

// ===========================================================================
// TauriIpcTsConnector — TypeScript start-side (invoke() calls + listen())
// ===========================================================================

pub struct TauriIpcTsConnector;

impl Connector for TauriIpcTsConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "tauri_ipc_ts",
            protocols: &[Protocol::Ipc],
            languages: &["typescript", "tsx", "javascript", "jsx"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.rust_crates.contains("tauri")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();

        // invoke("command_name") call sites → Start points
        let calls = ts_find_invoke_calls(conn, project_root)
            .context("Tauri invoke call detection failed")?;

        for call in &calls {
            points.push(ConnectionPoint {
                file_id: call.file_id,
                symbol_id: None,
                line: call.line,
                protocol: Protocol::Ipc,
                direction: FlowDirection::Start,
                key: call.command_name.clone(),
                method: String::new(),
                framework: "tauri".to_string(),
                metadata: None,
            });
        }

        // listen("event-name") sites → Stop points for events
        extract_tauri_listen_events(conn, project_root, &mut points)?;

        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Tauri IPC TS-side helpers (inlined from connectors/tauri_ipc.rs)
// ---------------------------------------------------------------------------

struct TsInvokeCall {
    file_id: i64,
    line: u32,
    command_name: String,
}

fn ts_find_invoke_calls(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<TsInvokeCall>> {
    let re_invoke = regex::Regex::new(
        r#"invoke\s*(?:<[^>]*>)?\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
    ).expect("invoke regex is valid");

    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
        )
        .context("Failed to prepare TS/JS files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TS/JS files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TS/JS file rows")?;

    let mut calls = Vec::new();
    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable TS file");
                continue;
            }
        };
        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re_invoke.captures_iter(line_text) {
                let name = cap
                    .name("name1")
                    .or_else(|| cap.name("name2"))
                    .or_else(|| cap.name("name3"))
                    .map(|m| m.as_str().to_string());
                if let Some(command_name) = name {
                    calls.push(TsInvokeCall { file_id, line: line_no, command_name });
                }
            }
        }
    }
    debug!(count = calls.len(), "invoke() calls found");
    Ok(calls)
}

fn extract_tauri_listen_events(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let re_listen = regex::Regex::new(
        r#"(?:\.\s*)?listen\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
    )
    .expect("listen regex");

    for lang in &["typescript", "tsx", "javascript", "jsx"] {
        scan_for_ipc_pattern(conn, project_root, lang, &re_listen, FlowDirection::Stop, "tauri", out)?;
    }

    Ok(())
}

// ===========================================================================
// ElectronIpcConnector — TypeScript/JavaScript only
// ===========================================================================

pub struct ElectronIpcConnector;

impl Connector for ElectronIpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "electron_ipc",
            protocols: &[Protocol::Ipc],
            languages: &["typescript", "tsx", "javascript", "jsx"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.ts_packages.contains("electron")
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let re_main = regex::Regex::new(
            r#"ipcMain\s*\.\s*(?:handle|on)\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
        )
        .expect("ipcMain regex");
        let re_renderer = regex::Regex::new(
            r#"ipcRenderer\s*\.\s*(?:invoke|send)\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)'|`(?P<name3>[^`]+)`)"#,
        )
        .expect("ipcRenderer regex");

        let mut points = Vec::new();

        // ipcMain.handle/on → Stop (handler side)
        for lang in &["typescript", "tsx", "javascript", "jsx"] {
            scan_for_ipc_pattern(conn, project_root, lang, &re_main, FlowDirection::Stop, "electron", &mut points)?;
        }

        // ipcRenderer.invoke/send → Start (caller side)
        for lang in &["typescript", "tsx", "javascript", "jsx"] {
            scan_for_ipc_pattern(conn, project_root, lang, &re_renderer, FlowDirection::Start, "electron", &mut points)?;
        }

        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Shared IPC scan helper
// ---------------------------------------------------------------------------

/// Scan all files of a given language for a regex pattern that captures a name
/// (via named groups name1/name2/name3) and emit IPC ConnectionPoints.
fn scan_for_ipc_pattern(
    conn: &Connection,
    project_root: &Path,
    language: &str,
    re: &regex::Regex,
    direction: FlowDirection,
    framework: &str,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = ?1")
        .context("Failed to prepare file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([language], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .context("Failed to query files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect file rows")?;

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            for cap in re.captures_iter(line_text) {
                let name = cap
                    .name("name1")
                    .or_else(|| cap.name("name2"))
                    .or_else(|| cap.name("name3"))
                    .map(|m| m.as_str().to_string());

                if let Some(key) = name {
                    out.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Ipc,
                        direction,
                        key,
                        method: String::new(),
                        framework: framework.to_string(),
                        metadata: None,
                    });
                }
            }
        }
    }

    Ok(())
}

// ===========================================================================
// React patterns post-index hook + inlined helpers
// ===========================================================================

/// Detect React-ecosystem patterns (Zustand stores, Storybook stories) and
/// create concept entries.
///
/// Called from `TypeScriptPlugin::post_index()`. Non-fatal — each sub-step
/// logs warnings on failure rather than propagating errors.
pub fn run_react_patterns(conn: &rusqlite::Connection, project_root: &std::path::Path) {
    use tracing::warn;

    match react_find_zustand_stores(conn, project_root) {
        Ok(stores) => {
            match react_find_story_mappings(conn, project_root) {
                Ok(stories) if !stores.is_empty() || !stories.is_empty() => {
                    let _ = react_create_concepts(conn, &stores, &stories)
                        .map_err(|e| warn!("React concept creation: {e}"));
                }
                Err(e) => warn!("Story mapping: {e}"),
                _ => {}
            }
        }
        Err(e) => warn!("Zustand store detection: {e}"),
    }
}

// ---------------------------------------------------------------------------
// React patterns helpers (inlined from connectors/react_patterns.rs)
// ---------------------------------------------------------------------------

/// A Zustand store definition in a TypeScript file.
#[derive(Debug, Clone)]
pub struct ZustandStore {
    pub file_id: i64,
    pub name: String,
    pub line: u32,
}

/// The mapping between a Storybook story file and its component.
#[derive(Debug, Clone)]
pub struct StoryMapping {
    pub story_file_id: i64,
    pub component_name: String,
    pub component_file_path: Option<String>,
}

fn react_find_zustand_stores(
    conn: &rusqlite::Connection,
    project_root: &std::path::Path,
) -> anyhow::Result<Vec<ZustandStore>> {
    let re_store = regex::Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
        .expect("store regex is valid");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language IN ('typescript', 'tsx')")
        .context("Failed to prepare TS files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TS files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TS file rows")?;

    let mut stores = Vec::new();
    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable TS file");
                continue;
            }
        };
        for (line_idx, line_text) in source.lines().enumerate() {
            if let Some(cap) = re_store.captures(line_text) {
                stores.push(ZustandStore {
                    file_id,
                    name: cap[1].to_string(),
                    line: (line_idx + 1) as u32,
                });
            }
        }
    }
    debug!(count = stores.len(), "Zustand stores found");
    Ok(stores)
}

fn react_find_story_mappings(
    conn: &rusqlite::Connection,
    project_root: &std::path::Path,
) -> anyhow::Result<Vec<StoryMapping>> {
    let re_default_export = regex::Regex::new(r"component\s*:\s*(\w+)")
        .expect("default export regex is valid");
    let re_meta_type = regex::Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>")
        .expect("meta type regex is valid");

    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE (path LIKE '%.stories.tsx' OR path LIKE '%.stories.ts')
               AND language IN ('typescript', 'tsx')",
        )
        .context("Failed to prepare story files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query story files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect story file rows")?;

    let mut mappings = Vec::new();
    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable story file");
                continue;
            }
        };

        let component_name = react_extract_component_name(&source, &re_default_export, &re_meta_type);
        let component_name = match component_name {
            Some(n) => n,
            None => {
                debug!(path = %rel_path, "Could not extract component name from story file");
                continue;
            }
        };

        let component_file_path: Option<String> = conn
            .query_row(
                "SELECT f.path FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.name = ?1 AND s.kind = 'class' OR s.kind = 'function'
                 LIMIT 1",
                [&component_name],
                |r| r.get(0),
            )
            .optional();

        mappings.push(StoryMapping {
            story_file_id: file_id,
            component_name,
            component_file_path,
        });
    }
    debug!(count = mappings.len(), "Story mappings found");
    Ok(mappings)
}

fn react_extract_component_name(
    source: &str,
    re_default: &regex::Regex,
    re_meta: &regex::Regex,
) -> Option<String> {
    for line in source.lines() {
        if let Some(cap) = re_meta.captures(line) {
            return Some(cap[1].to_string());
        }
    }
    for line in source.lines() {
        if let Some(cap) = re_default.captures(line) {
            return Some(cap[1].to_string());
        }
    }
    None
}

fn react_create_concepts(
    conn: &rusqlite::Connection,
    stores: &[ZustandStore],
    stories: &[StoryMapping],
) -> anyhow::Result<()> {
    if !stores.is_empty() {
        let concept_id = react_upsert_concept(conn, "zustand-stores", "Zustand state stores")?;
        for store in stores {
            let symbol_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM symbols WHERE file_id = ?1 AND name = ?2 LIMIT 1",
                    rusqlite::params![store.file_id, store.name],
                    |r| r.get(0),
                )
                .optional();
            if let Some(sym_id) = symbol_id {
                react_add_concept_member(conn, concept_id, sym_id)?;
            } else {
                debug!(store = %store.name, "Zustand store symbol not found in index — concept member not added");
            }
        }
        tracing::info!(stores = stores.len(), "React patterns: zustand-stores concept updated");
    }

    if !stories.is_empty() {
        let concept_id = react_upsert_concept(conn, "storybook-stories", "Storybook story files")?;
        for story in stories {
            let symbol_ids: Vec<i64> = {
                let mut stmt = conn
                    .prepare("SELECT id FROM symbols WHERE file_id = ?1")
                    .context("Failed to prepare story symbol query")?;
                let rows: rusqlite::Result<Vec<i64>> =
                    stmt.query_map([story.story_file_id], |r| r.get(0))?.collect();
                rows.context("Failed to collect story symbol ids")?
            };
            for sym_id in symbol_ids {
                react_add_concept_member(conn, concept_id, sym_id)?;
            }
        }
        tracing::info!(stories = stories.len(), "React patterns: storybook-stories concept updated");
    }

    Ok(())
}

fn react_upsert_concept(conn: &rusqlite::Connection, name: &str, description: &str) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO concepts (name, description) VALUES (?1, ?2)",
        rusqlite::params![name, description],
    ).context("Failed to upsert concept")?;
    let id: i64 = conn
        .query_row("SELECT id FROM concepts WHERE name = ?1", [name], |r| r.get(0))
        .context("Failed to fetch concept id")?;
    Ok(id)
}

fn react_add_concept_member(conn: &rusqlite::Connection, concept_id: i64, symbol_id: i64) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO concept_members (concept_id, symbol_id, auto_assigned)
         VALUES (?1, ?2, 1)",
        rusqlite::params![concept_id, symbol_id],
    ).context("Failed to insert concept member")?;
    Ok(())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn controller_regex_with_single_quoted_prefix() {
        let re = NestRegexes::build();
        let line = "@Controller('users')";
        let cap = re.controller.captures(line).unwrap();
        assert_eq!(cap.get(1).unwrap().as_str(), "users");
    }

    #[test]
    fn controller_regex_with_double_quoted_prefix() {
        let re = NestRegexes::build();
        let line = r#"@Controller("articles")"#;
        let cap = re.controller.captures(line).unwrap();
        assert_eq!(cap.get(1).unwrap().as_str(), "articles");
    }

    #[test]
    fn controller_regex_with_no_argument() {
        let re = NestRegexes::build();
        let line = "@Controller()";
        let cap = re.controller.captures(line).unwrap();
        assert!(cap.get(1).is_none(), "no-arg controller should have no prefix group");
    }

    #[test]
    fn method_decorator_regex_get_no_arg() {
        let re = NestRegexes::build();
        let line = "  @Get()";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Get");
        assert!(cap.get(2).is_none());
    }

    #[test]
    fn method_decorator_regex_get_with_param() {
        let re = NestRegexes::build();
        let line = "  @Get(':id')";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Get");
        assert_eq!(cap.get(2).unwrap().as_str(), ":id");
    }

    #[test]
    fn method_decorator_regex_post_double_quoted() {
        let re = NestRegexes::build();
        let line = r#"  @Post("register")"#;
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Post");
        assert_eq!(cap.get(2).unwrap().as_str(), "register");
    }

    #[test]
    fn method_decorator_regex_delete_with_param() {
        let re = NestRegexes::build();
        let line = "  @Delete(':id')";
        let cap = re.method_decorator.captures(line).unwrap();
        assert_eq!(&cap[1], "Delete");
        assert_eq!(cap.get(2).unwrap().as_str(), ":id");
    }

    #[test]
    fn join_paths_prefix_and_suffix() {
        assert_eq!(nest_join_paths("/users", ":id"), "/users/:id");
        assert_eq!(nest_join_paths("/users/", "/:id"), "/users/:id");
    }

    #[test]
    fn join_paths_no_suffix() {
        assert_eq!(nest_join_paths("/users", ""), "/users");
    }

    #[test]
    fn join_paths_no_prefix() {
        assert_eq!(nest_join_paths("", "register"), "/register");
    }

    #[test]
    fn join_paths_both_empty() {
        assert_eq!(nest_join_paths("", ""), "/");
    }

    #[test]
    fn normalise_prefix_adds_leading_slash() {
        assert_eq!(normalise_prefix("users"), "/users");
    }

    #[test]
    fn normalise_prefix_strips_trailing_slash() {
        assert_eq!(normalise_prefix("/users/"), "/users");
    }

    #[test]
    fn normalise_prefix_empty_stays_empty() {
        assert_eq!(normalise_prefix(""), "");
    }

    #[test]
    fn extracts_basic_controller_and_get_method() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/users/users.controller.ts', 'h1', 'typescript', 0)",
            [],
        )
        .unwrap();

        let source = r#"
import { Controller, Get } from '@nestjs/common';

@Controller('users')
export class UsersController {
  @Get()
  findAll() {
    return this.usersService.findAll();
  }
}
"#;
        let re = NestRegexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(conn, source, 1, "src/users/users.controller.ts", &re, &mut routes);

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "GET");
        assert_eq!(routes[0].route_template, "/users");
        assert_eq!(routes[0].handler_name, "findAll");
    }

    #[test]
    fn extracts_parameterised_route() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/articles/articles.controller.ts', 'h2', 'typescript', 0)",
            [],
        )
        .unwrap();

        let source = r#"
import { Controller, Get, Post, Delete } from '@nestjs/common';

@Controller('articles')
export class ArticlesController {
  @Get(':slug')
  findOne(@Param('slug') slug: string) {
    return this.articlesService.findBySlug(slug);
  }

  @Post()
  create(@Body() dto: CreateArticleDto) {
    return this.articlesService.create(dto);
  }

  @Delete(':id')
  remove(@Param('id') id: string) {
    return this.articlesService.remove(id);
  }
}
"#;
        let re = NestRegexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(
            conn,
            source,
            1,
            "src/articles/articles.controller.ts",
            &re,
            &mut routes,
        );

        assert_eq!(routes.len(), 3);

        let get = routes.iter().find(|r| r.http_method == "GET").unwrap();
        assert_eq!(get.route_template, "/articles/:slug");
        assert_eq!(get.handler_name, "findOne");

        let post = routes.iter().find(|r| r.http_method == "POST").unwrap();
        assert_eq!(post.route_template, "/articles");
        assert_eq!(post.handler_name, "create");

        let delete = routes.iter().find(|r| r.http_method == "DELETE").unwrap();
        assert_eq!(delete.route_template, "/articles/:id");
        assert_eq!(delete.handler_name, "remove");
    }

    #[test]
    fn extracts_controller_with_no_prefix() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/health/health.controller.ts', 'h3', 'typescript', 0)",
            [],
        )
        .unwrap();

        let source = r#"
import { Controller, Get } from '@nestjs/common';

@Controller()
export class HealthController {
  @Get('healthz')
  check() {
    return { status: 'ok' };
  }
}
"#;
        let re = NestRegexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(
            conn,
            source,
            1,
            "src/health/health.controller.ts",
            &re,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].http_method, "GET");
        assert_eq!(routes[0].route_template, "/healthz");
        assert_eq!(routes[0].handler_name, "check");
    }

    #[test]
    fn extracts_all_http_verbs() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/items/items.controller.ts', 'h4', 'typescript', 0)",
            [],
        )
        .unwrap();

        let source = r#"
@Controller('items')
export class ItemsController {
  @Get()
  findAll() {}

  @Get(':id')
  findOne() {}

  @Post()
  create() {}

  @Put(':id')
  update() {}

  @Patch(':id')
  patch() {}

  @Delete(':id')
  remove() {}
}
"#;
        let re = NestRegexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(conn, source, 1, "src/items/items.controller.ts", &re, &mut routes);

        assert_eq!(routes.len(), 6);
        let methods: Vec<&str> = routes.iter().map(|r| r.http_method.as_str()).collect();
        assert!(methods.contains(&"GET"));
        assert!(methods.contains(&"POST"));
        assert!(methods.contains(&"PUT"));
        assert!(methods.contains(&"PATCH"));
        assert!(methods.contains(&"DELETE"));
    }

    #[test]
    fn symbol_id_is_resolved_when_indexed() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/users/users.controller.ts', 'h1', 'typescript', 0)",
            [],
        )
        .unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'findAll', 'UsersController.findAll', 'method', 7, 2)",
            [file_id],
        )
        .unwrap();
        let sym_id: i64 = conn.last_insert_rowid();

        let source = r#"
@Controller('users')
export class UsersController {
  @Get()
  findAll() {
    return [];
  }
}
"#;
        let re = NestRegexes::build();
        let mut routes = Vec::new();
        extract_routes_from_source(
            conn,
            source,
            file_id,
            "src/users/users.controller.ts",
            &re,
            &mut routes,
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].symbol_id, Some(sym_id));
    }
}

#[cfg(test)]
mod react_patterns_tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn store_regex_matches_export_const() {
        let re = regex::Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
            .unwrap();
        let line = "export const useEditorStore = create<EditorState>((set) => ({";
        assert!(re.is_match(line));
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "useEditorStore");
    }

    #[test]
    fn store_regex_matches_const_without_export() {
        let re = regex::Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
            .unwrap();
        let line = "const useAuthStore = create(initializer)";
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "useAuthStore");
    }

    #[test]
    fn store_regex_does_not_match_non_use_prefix() {
        let re = regex::Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
            .unwrap();
        assert!(!re.is_match("const myState = create<State>()"));
    }

    #[test]
    fn extract_component_name_from_meta_type() {
        let re_default = regex::Regex::new(r"component\s*:\s*(\w+)").unwrap();
        let re_meta = regex::Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>").unwrap();
        let source = "const meta: Meta<typeof Button> = { title: 'Button' };\nexport default meta;";
        let name = react_extract_component_name(source, &re_default, &re_meta);
        assert_eq!(name, Some("Button".to_string()));
    }

    #[test]
    fn extract_component_name_from_default_export() {
        let re_default = regex::Regex::new(r"component\s*:\s*(\w+)").unwrap();
        let re_meta = regex::Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>").unwrap();
        let source = "export default { component: FileTree, title: 'FileTree' };";
        let name = react_extract_component_name(source, &re_default, &re_meta);
        assert_eq!(name, Some("FileTree".to_string()));
    }

    #[test]
    fn extract_component_name_meta_takes_priority() {
        let re_default = regex::Regex::new(r"component\s*:\s*(\w+)").unwrap();
        let re_meta = regex::Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>").unwrap();
        let source = "const meta: Meta<typeof Button> = { component: OtherThing };";
        let name = react_extract_component_name(source, &re_default, &re_meta);
        assert_eq!(name, Some("Button".to_string()));
    }

    #[test]
    fn react_create_concepts_adds_zustand_concept() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/stores/editorStore.ts', 'h1', 'typescript', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'useEditorStore', 'useEditorStore', 'variable', 3, 0)",
            [file_id],
        ).unwrap();

        let stores = vec![ZustandStore { file_id, name: "useEditorStore".to_string(), line: 3 }];
        react_create_concepts(conn, &stores, &[]).unwrap();

        let concept_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM concepts WHERE name = 'zustand-stores'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(concept_count, 1);
    }

    #[test]
    fn react_create_concepts_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/stores/editorStore.ts', 'h1', 'typescript', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'useEditorStore', 'useEditorStore', 'variable', 3, 0)",
            [file_id],
        ).unwrap();

        let stores = vec![ZustandStore { file_id, name: "useEditorStore".to_string(), line: 3 }];
        react_create_concepts(conn, &stores, &[]).unwrap();
        react_create_concepts(conn, &stores, &[]).unwrap();

        let member_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM concept_members", [], |r| r.get(0))
            .unwrap();
        assert_eq!(member_count, 1);
    }

    #[test]
    fn empty_inputs_produce_no_concepts() {
        let db = Database::open_in_memory().unwrap();
        react_create_concepts(db.conn(), &[], &[]).unwrap();
        let count: i64 = db.conn()
            .query_row("SELECT COUNT(*) FROM concepts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

// ===========================================================================
// TypeScriptRestConnector — HTTP client call starts + route stops for TS/JS
// ===========================================================================

pub struct TypeScriptRestConnector;

impl Connector for TypeScriptRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "typescript_rest",
            protocols: &[Protocol::Rest],
            languages: &["typescript", "tsx", "javascript", "jsx"],
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
        extract_ts_rest_stops(conn, &mut points)?;
        extract_ts_rest_starts(conn, project_root, &mut points)?;
        Ok(points)
    }
}

fn extract_ts_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language IN ('typescript', 'tsx', 'javascript', 'jsx')
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare TS REST stops query")?;

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
        .context("Failed to query TS routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read TS route row")?;
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

fn extract_ts_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let re_fetch = Regex::new(
        r#"fetch\s*\(\s*(?:\w+\s*\(\s*)?(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)'|`(?P<url3>[^`]+)`)"#,
    )
    .expect("fetch regex");
    let re_axios = Regex::new(
        r#"axios\.(?P<method>get|post|put|delete|patch|head)\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)'|`(?P<url3>[^`]+)`)"#,
    )
    .expect("axios regex");
    let re_method_extract = Regex::new(r#"method\s*:\s*['"](?P<m>[A-Z]+)['"]"#)
        .expect("method extract regex");

    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
        )
        .context("Failed to prepare TS files query")?;
    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TS files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TS file rows")?;

    for (file_id, rel_path) in files {
        if ts_rest_is_test_or_config_file(&rel_path) {
            continue;
        }
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;

            for cap in re_fetch.captures_iter(line_text) {
                let raw_url = cap
                    .name("url1")
                    .or_else(|| cap.name("url2"))
                    .or_else(|| cap.name("url3"))
                    .map(|m| m.as_str().to_string());
                let Some(raw_url) = raw_url else { continue };
                if !ts_rest_looks_like_api_url(&raw_url) {
                    continue;
                }
                let method = re_method_extract
                    .captures(line_text)
                    .and_then(|c| c.name("m"))
                    .map(|m| m.as_str().to_string())
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

            for cap in re_axios.captures_iter(line_text) {
                let raw_url = cap
                    .name("url1")
                    .or_else(|| cap.name("url2"))
                    .or_else(|| cap.name("url3"))
                    .map(|m| m.as_str().to_string());
                let Some(raw_url) = raw_url else { continue };
                if !ts_rest_looks_like_api_url(&raw_url) {
                    continue;
                }
                let method = cap
                    .name("method")
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

fn ts_rest_is_test_or_config_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.contains("_test.")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.contains(".config.")
        || lower.contains("__tests__")
        || lower.contains("/node_modules/")
        || lower.contains("/vendor/")
        || lower.ends_with(".min.js")
        || lower.contains("/e2e/")
        || lower.contains("/cypress/")
}

fn ts_rest_looks_like_api_url(s: &str) -> bool {
    // For TS/JS frontend calls, reject absolute URLs (they don't match local stops)
    if s.starts_with("http://") || s.starts_with("https://") {
        return false;
    }
    let lower = s.to_lowercase();
    if let Some(last_seg) = lower.rsplit('/').next() {
        if last_seg.contains('.') {
            let ext = lower.rsplit('.').next().unwrap_or("");
            if matches!(
                ext,
                "svg" | "png" | "jpg" | "jpeg" | "gif" | "ico" | "webp"
                    | "css" | "js" | "html" | "htm" | "xml" | "json"
                    | "txt" | "md"
            ) {
                return false;
            }
        }
    }
    s.starts_with('/')
        || s.contains("/api/")
        || s.contains("/v1/")
        || s.contains("/v2/")
        || s.contains("/v3/")
        || s.starts_with("api/")
        || s.contains("/${")
        || s.contains("/{")
}

fn rest_normalise_url_pattern(raw: &str) -> String {
    let without_query = raw.split('?').next().unwrap_or(raw);
    let re_tmpl = Regex::new(r"\$\{[^}]+\}").expect("template regex");
    re_tmpl.replace_all(without_query, "{param}").into_owned()
}

// ===========================================================================
// TypeScriptMqConnector — Message queue producer/consumer connection points
// ===========================================================================

/// Detects TypeScript/JavaScript message queue patterns for:
///   - kafkajs: `producer.send({ topic: "name", ... })` (producer)
///              `consumer.subscribe({ topic: "name" })` (consumer)
///              `@Subscribe("topic")` decorator (consumer)
///   - amqplib (RabbitMQ): `channel.publish("exchange", "routingKey", ...)` (producer)
///                          `channel.consume("queue", ...)` (consumer)
///   - bullmq / bull: `new Queue("queue-name")` + `@Processor("queue-name")` (consumer)
pub struct TypeScriptMqConnector;

impl Connector for TypeScriptMqConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "typescript_mq",
            protocols: &[Protocol::MessageQueue],
            languages: &["typescript", "tsx", "javascript", "jsx"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.ts_packages.contains("kafkajs")
            || ctx.ts_packages.contains("kafka-node")
            || ctx.ts_packages.contains("amqplib")
            || ctx.ts_packages.contains("amqp-connection-manager")
            || ctx.ts_packages.contains("bullmq")
            || ctx.ts_packages.contains("bull")
            || ctx.ts_packages.contains("@nestjs/microservices")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // kafkajs: producer.send({ topic: "name" ...})
        let re_kafka_send = Regex::new(
            r#"producer\.send\s*\(\s*\{[^}]*topic\s*:\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("ts kafka send regex");

        // kafkajs: consumer.subscribe({ topic: "name" }) or consumer.subscribe({ topics: ["name"] })
        let re_kafka_subscribe = Regex::new(
            r#"consumer\.subscribe\s*\(\s*\{[^}]*topics?\s*:\s*(?:\[[^\]]*['"`]([^'"`]+)['"`]|['"`]([^'"`]+)['"`])"#,
        )
        .expect("ts kafka subscribe regex");

        // amqplib: channel.publish("exchange", "routingKey", ...)
        let re_amqp_publish = Regex::new(
            r#"channel\.publish\s*\(\s*['"`]([^'"`]+)['"`]\s*,\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("ts amqp publish regex");

        // amqplib: channel.consume("queue", ...)
        let re_amqp_consume = Regex::new(
            r#"channel\.consume\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("ts amqp consume regex");

        // BullMQ / NestJS @Processor("queue") decorator
        let re_processor = Regex::new(
            r#"@Processor\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("ts bullmq processor regex");

        // BullMQ: new Queue("queue-name") or new Worker("queue-name", ...)
        let re_queue_ctor = Regex::new(
            r#"new\s+Queue\s*\(\s*['"`]([^'"`]+)['"`]"#,
        )
        .expect("ts bullmq queue ctor regex");

        let mut stmt = conn
            .prepare(
                "SELECT id, path FROM files
                 WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
            )
            .context("Failed to prepare TS/JS files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .context("Failed to query TS/JS files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect TS/JS file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
            let lower = rel_path.to_lowercase();
            if lower.contains("/node_modules/") || lower.ends_with(".min.js") {
                continue;
            }

            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                for cap in re_kafka_send.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "kafka".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_kafka_subscribe.captures_iter(line_text) {
                    let topic = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str());
                    if let Some(t) = topic {
                        points.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::MessageQueue,
                            direction: FlowDirection::Stop,
                            key: t.to_string(),
                            method: String::new(),
                            framework: "kafka".to_string(),
                            metadata: None,
                        });
                    }
                }

                for cap in re_amqp_publish.captures_iter(line_text) {
                    // Use routing key as the topic key.
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[2].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_amqp_consume.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_processor.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Stop,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "bullmq".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_queue_ctor.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "bullmq".to_string(),
                        metadata: None,
                    });
                }
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// TypeScriptGraphQlConnector — GraphQL schema starts + resolver stops
// ===========================================================================

/// Detects TypeScript/JavaScript GraphQL operations and resolver implementations.
///
/// Start points: `gql` template literals containing `type Query { ... }`,
///               `type Mutation { ... }`, or `type Subscription { ... }`.
/// Stop points:  resolver map functions (Apollo Server, Mercurius, GraphQL Yoga).
pub struct TypeScriptGraphQlConnector;

impl Connector for TypeScriptGraphQlConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "typescript_graphql",
            protocols: &[Protocol::GraphQl],
            languages: &["typescript", "tsx", "javascript", "jsx"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.ts_packages.contains("graphql")
            || ctx.ts_packages.contains("@apollo/server")
            || ctx.ts_packages.contains("apollo-server")
            || ctx.ts_packages.contains("@apollo/client")
            || ctx.ts_packages.contains("mercurius")
            || ctx.ts_packages.contains("graphql-yoga")
            || ctx.ts_packages.contains("type-graphql")
            || ctx.ts_packages.contains("nexus")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // GraphQL type blocks inside SDL or gql`` literals.
        let re_type_block = Regex::new(r"type\s+(Query|Mutation|Subscription)\s*\{")
            .expect("gql type block regex");
        let re_field = Regex::new(r"^\s+(\w+)(?:\([^)]*\))?\s*:")
            .expect("gql field regex");

        // Apollo/GraphQL resolver map: resolvers.Query = { fieldName(...) }
        // or:  Query: { fieldName: (_parent, _args, ctx) => ... }
        let re_resolver_key = Regex::new(
            r#"['"`]?(\w+)['"`]?\s*:\s*(?:async\s+)?\([^)]*\)\s*=>"#,
        )
        .expect("ts graphql resolver key regex");

        // @Resolver() class + @Query() / @Mutation() — type-graphql
        let re_typegraphql_op = Regex::new(
            r#"@(?:Query|Mutation|Subscription)\s*\(\s*\([^)]*\)\s*=>\s*\w+\s*\)"#,
        )
        .expect("ts type-graphql op regex");

        let mut stmt = conn
            .prepare(
                "SELECT id, path, language FROM files
                 WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
            )
            .context("Failed to prepare TS/JS files query")?;

        let files: Vec<(i64, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .context("Failed to query TS/JS files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect TS/JS file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path, _lang) in files {
            let lower = rel_path.to_lowercase();
            if lower.contains("/node_modules/") || lower.ends_with(".min.js") {
                continue;
            }

            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Quick filter: skip files that contain no GraphQL SDL.
            if !re_type_block.is_match(&source) && !source.contains("@Query") && !source.contains("@Mutation") {
                continue;
            }

            extract_ts_graphql_points(
                &source,
                file_id,
                &re_type_block,
                &re_field,
                &re_resolver_key,
                &re_typegraphql_op,
                &mut points,
            );
        }

        Ok(points)
    }
}

fn extract_ts_graphql_points(
    source: &str,
    file_id: i64,
    re_type_block: &Regex,
    re_field: &Regex,
    re_resolver_key: &Regex,
    re_typegraphql_op: &Regex,
    out: &mut Vec<ConnectionPoint>,
) {
    let mut current_op_type: Option<String> = None;
    let mut brace_depth: u32 = 0;

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // Detect GraphQL type blocks to emit Start points.
        if let Some(cap) = re_type_block.captures(line_text) {
            current_op_type = Some(cap[1].to_lowercase());
            brace_depth = 1;
            continue;
        }

        if let Some(ref _op_type) = current_op_type {
            for ch in line_text.chars() {
                match ch {
                    '{' => brace_depth += 1,
                    '}' => {
                        if brace_depth > 0 {
                            brace_depth -= 1;
                        }
                    }
                    _ => {}
                }
            }

            if brace_depth == 0 {
                current_op_type = None;
                continue;
            }

            if brace_depth == 1 {
                if let Some(cap) = re_field.captures(line_text) {
                    let field_name = cap[1].to_string();
                    if !field_name.starts_with("__") {
                        out.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::GraphQl,
                            direction: FlowDirection::Start,
                            key: field_name,
                            method: current_op_type.clone().unwrap_or_default(),
                            framework: String::new(),
                            metadata: None,
                        });
                    }
                }
            }
            continue;
        }

        // Resolver map entries — Stop points.
        for cap in re_resolver_key.captures_iter(line_text) {
            let name = cap[1].to_string();
            // Exclude common non-resolver patterns.
            if matches!(
                name.as_str(),
                "then" | "catch" | "finally" | "map" | "filter" | "reduce"
            ) {
                continue;
            }
            out.push(ConnectionPoint {
                file_id,
                symbol_id: None,
                line: line_no,
                protocol: Protocol::GraphQl,
                direction: FlowDirection::Stop,
                key: name,
                method: String::new(),
                framework: "apollo".to_string(),
                metadata: None,
            });
        }

        // type-graphql @Query() / @Mutation() decorators.
        if re_typegraphql_op.is_match(line_text) {
            out.push(ConnectionPoint {
                file_id,
                symbol_id: None,
                line: line_no,
                protocol: Protocol::GraphQl,
                direction: FlowDirection::Stop,
                key: String::new(), // name resolved from next function
                method: String::new(),
                framework: "type-graphql".to_string(),
                metadata: None,
            });
        }
    }
}
