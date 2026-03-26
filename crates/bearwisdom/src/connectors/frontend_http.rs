// =============================================================================
// connectors/frontend_http.rs  —  Detect HTTP calls in TypeScript / JS and
//                                  other languages
//
// Three jobs:
//   1. `detect_http_calls` — scan TS/JS files for fetch() and axios.* calls,
//      extract the URL and HTTP method, return a list of `DetectedHttpCall`.
//   2. `match_http_calls_to_routes` — match detected calls against backend
//      routes in the `routes` table and insert `flow_edges`.
//   3. `detect_http_calls_all_languages` — extends job 1 with detection for
//      Python (requests / httpx), Go (http.Get / http.Post / http.NewRequest),
//      Java/Kotlin (HttpClient / RestTemplate / WebClient), Ruby
//      (Net::HTTP / Faraday / HTTParty), and C# (HttpClient.*Async).
//
// Detection strategy:
//   Uses a regex-based first pass.  Tree-sitter could give precise AST nodes
//   but requires resolving source file bytes through the indexer pipeline.  The
//   regex pass is robust enough for common patterns and avoids the bootstrap
//   dependency.
//
// TODO: upgrade to tree-sitter-based detection for full AST accuracy.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::debug;

use crate::connectors::http_api::{normalise_route, routes_match};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// An HTTP call site detected in a TypeScript / JavaScript file.
#[derive(Debug, Clone)]
pub struct DetectedHttpCall {
    /// `files.id` of the file containing the call.
    pub file_id: i64,
    /// `symbols.id` of the containing function/method, if known.
    pub symbol_id: Option<i64>,
    /// 1-based line number of the call.
    pub line: u32,
    /// Normalised HTTP method ("GET", "POST", "PUT", "DELETE", "PATCH").
    pub http_method: String,
    /// Normalised URL pattern — template literals collapsed to `{param}`,
    /// query strings stripped.
    pub url_pattern: String,
    /// The raw URL string as it appears in the source.
    pub raw_url: String,
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Scan all TypeScript / JavaScript files for `fetch()` and `axios.*` calls.
///
/// Reads each file from disk using `project_root` as the base directory.
/// Returns detected HTTP calls with extracted URL patterns.
pub fn detect_http_calls(conn: &Connection, project_root: &Path) -> Result<Vec<DetectedHttpCall>> {
    // Compile patterns once.
    // fetch("url") or fetch('url') or fetch(`url`) — with optional method in options
    let re_fetch = build_fetch_regex();
    // axios.get("url"), axios.post("url"), etc.
    let re_axios = build_axios_regex();

    // Query all TS/JS files.
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language IN ('typescript', 'tsx', 'javascript', 'jsx')",
        )
        .context("Failed to prepare frontend file query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query frontend files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect frontend file rows")?;

    let mut calls: Vec<DetectedHttpCall> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable file");
                continue;
            }
        };

        detect_in_source(&source, file_id, &re_fetch, &re_axios, &mut calls);
    }

    Ok(calls)
}

/// Match detected frontend HTTP calls to backend route definitions.
///
/// For each detected call we find the best-matching route by HTTP method +
/// URL pattern similarity, then insert a `flow_edges` row linking the frontend
/// call site to the backend handler.  Returns the number of edges created.
pub fn match_http_calls_to_routes(
    conn: &Connection,
    calls: &[DetectedHttpCall],
) -> Result<u32> {
    if calls.is_empty() {
        return Ok(0);
    }

    // Load all routes.
    let routes = load_routes(conn)?;
    if routes.is_empty() {
        return Ok(0);
    }

    let mut created: u32 = 0;

    for call in calls {
        let call_norm = normalise_route(&call.url_pattern);

        // Find routes that match by method + normalised URL.
        let matched: Vec<&RouteRow> = routes
            .iter()
            .filter(|r| {
                // Method must match (or the call is a generic "GET" default).
                let method_ok =
                    r.http_method.eq_ignore_ascii_case(&call.http_method);
                if !method_ok {
                    return false;
                }
                let route_norm = normalise_route(
                    r.resolved_route.as_deref().unwrap_or(&r.route_template),
                );
                routes_match(&call_norm, &route_norm)
            })
            .collect();

        for route in matched {
            // Confidence: 1.0 for exact match after normalisation, 0.8 for
            // parameter-wildcard match.
            let route_norm = normalise_route(
                route.resolved_route.as_deref().unwrap_or(&route.route_template),
            );
            let confidence: f64 = if call_norm == route_norm { 1.0 } else { 0.8 };

            let result = conn.execute(
                "INSERT OR IGNORE INTO flow_edges (
                    source_file_id, source_line, source_symbol, source_language,
                    target_file_id, target_line, target_symbol, target_language,
                    edge_type, protocol, http_method, url_pattern, confidence
                 ) VALUES (
                    ?1, ?2, NULL, 'typescript',
                    ?3, ?4, ?5,   'csharp',
                    'http_call', 'http', ?6, ?7, ?8
                 )",
                rusqlite::params![
                    call.file_id,
                    call.line,
                    route.file_id,
                    route.line,
                    route.handler_name,
                    call.http_method,
                    call.url_pattern,
                    confidence,
                ],
            );

            match result {
                Ok(n) if n > 0 => created += 1,
                Ok(_) => {} // OR IGNORE hit — duplicate, skip
                Err(e) => {
                    debug!(err = %e, "Failed to insert flow_edge for http_call");
                }
            }
        }
    }

    Ok(created)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// A compact view of a route row for matching.
struct RouteRow {
    file_id: i64,
    line: Option<u32>,
    handler_name: Option<String>,
    http_method: String,
    route_template: String,
    resolved_route: Option<String>,
}

fn load_routes(conn: &Connection) -> Result<Vec<RouteRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.line, s.name, r.http_method,
                    r.route_template, r.resolved_route
             FROM routes r
             LEFT JOIN symbols s ON r.symbol_id = s.id",
        )
        .context("Failed to prepare routes query")?;

    let rows = stmt
        .query_map([], |row| {
            Ok(RouteRow {
                file_id: row.get::<_, i64>(0)?,
                line: row.get::<_, Option<u32>>(1)?,
                handler_name: row.get::<_, Option<String>>(2)?,
                http_method: row.get::<_, String>(3)?,
                route_template: row.get::<_, String>(4)?,
                resolved_route: row.get::<_, Option<String>>(5)?,
            })
        })
        .context("Failed to execute routes query")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect routes")?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Regex compilation
// ---------------------------------------------------------------------------

fn build_fetch_regex() -> Regex {
    // Matches: fetch("url"), fetch('url'), fetch(`url`)
    // Rust regex does not support backreferences, so we use alternation
    // for each quote type.
    Regex::new(
        r#"fetch\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)'|`(?P<url3>[^`]+)`)"#,
    )
    .expect("fetch regex is valid")
}

fn build_axios_regex() -> Regex {
    // Matches: axios.get("url"), axios.post('url'), axios.delete(`url`), etc.
    Regex::new(
        r#"axios\.(?P<method>get|post|put|delete|patch|head)\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)'|`(?P<url3>[^`]+)`)"#,
    )
    .expect("axios regex is valid")
}

/// Extract the URL from whichever quote-type capture group matched.
fn extract_url_from_captures(caps: &regex::Captures<'_>) -> Option<String> {
    caps.name("url1")
        .or_else(|| caps.name("url2"))
        .or_else(|| caps.name("url3"))
        .map(|m| m.as_str().to_string())
}

// ---------------------------------------------------------------------------
// Per-file detection
// ---------------------------------------------------------------------------

fn detect_in_source(
    source: &str,
    file_id: i64,
    re_fetch: &Regex,
    re_axios: &Regex,
    out: &mut Vec<DetectedHttpCall>,
) {
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // fetch(url) — method defaults to GET unless { method: "X" } follows.
        for cap in re_fetch.captures_iter(line_text) {
            if let Some(raw_url) = extract_url_from_captures(&cap) {
                let method = extract_fetch_method(line_text);
                let url_pattern = normalise_url_pattern(&raw_url);

                out.push(DetectedHttpCall {
                    file_id,
                    symbol_id: None,
                    line: line_no,
                    http_method: method,
                    url_pattern,
                    raw_url,
                });
            }
        }

        // axios.METHOD(url)
        for cap in re_axios.captures_iter(line_text) {
            let Some(raw_url) = extract_url_from_captures(&cap) else {
                continue;
            };
            let method = cap["method"].to_uppercase();
            let url_pattern = normalise_url_pattern(&raw_url);

            out.push(DetectedHttpCall {
                file_id,
                symbol_id: None,
                line: line_no,
                http_method: method,
                url_pattern,
                raw_url,
            });
        }
    }
}

/// Infer the HTTP method from a `fetch` call line.
///
/// Looks for `method: "POST"` or `method: 'DELETE'` in the same line.
/// Falls back to GET if no method option is found.
fn extract_fetch_method(line: &str) -> String {
    let re = Regex::new(r#"method\s*:\s*['"](?P<m>[A-Z]+)['"]"#).unwrap();
    if let Some(cap) = re.captures(line) {
        return cap["m"].to_string();
    }
    "GET".to_string()
}

/// Normalise a URL string for pattern matching:
///   - Replace `${...}` template literal expressions with `{param}`.
///   - Strip query string (`?...`).
///   - Preserve the path structure.
fn normalise_url_pattern(raw: &str) -> String {
    // Strip query string.
    let without_query = raw.split('?').next().unwrap_or(raw);

    // Replace template literal interpolations.
    let re_tmpl = Regex::new(r#"\$\{[^}]+\}"#).unwrap();
    re_tmpl.replace_all(without_query, "{param}").into_owned()
}

// ---------------------------------------------------------------------------
// Multi-language HTTP call detection
// ---------------------------------------------------------------------------

/// Language-specific regex patterns for HTTP calls.
struct LangPatterns {
    language: &'static str,
    /// Each entry: (compiled Regex, default method if not captured, method cap
    /// group name or None, url cap group name)
    matchers: Vec<LangMatcher>,
}

struct LangMatcher {
    re: Regex,
    /// Named capture group for the HTTP method, e.g. `"method"`.
    /// `None` means the method is implied by the regex (see `implied_method`).
    method_group: Option<&'static str>,
    /// Fallback method when `method_group` is None.
    implied_method: &'static str,
    /// Named capture group for the URL string.
    url_group: &'static str,
}

fn build_python_patterns() -> LangPatterns {
    LangPatterns {
        language: "python",
        matchers: vec![
            // requests.get("url"), requests.post("url"), etc.
            LangMatcher {
                re: Regex::new(
                    r#"requests\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
                )
                .expect("requests regex is valid"),
                method_group: Some("method"),
                implied_method: "GET",
                url_group: "url1", // fallback handled in scan
            },
            // httpx.get("url"), httpx.post("url"), etc.
            LangMatcher {
                re: Regex::new(
                    r#"httpx\s*\.\s*(?P<method>get|post|put|delete|patch)\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
                )
                .expect("httpx regex is valid"),
                method_group: Some("method"),
                implied_method: "GET",
                url_group: "url1",
            },
        ],
    }
}

fn build_go_patterns() -> LangPatterns {
    LangPatterns {
        language: "go",
        matchers: vec![
            // http.Get("url"), http.Post("url", ...)
            LangMatcher {
                re: Regex::new(
                    r#"http\s*\.\s*(?P<method>Get|Post)\s*\(\s*"(?P<url>[^"]+)""#,
                )
                .expect("go http.Get/Post regex is valid"),
                method_group: Some("method"),
                implied_method: "GET",
                url_group: "url",
            },
            // http.NewRequest("METHOD", "url", ...)
            LangMatcher {
                re: Regex::new(
                    r#"http\s*\.\s*NewRequest\s*\(\s*"(?P<method>[^"]+)"\s*,\s*"(?P<url>[^"]+)""#,
                )
                .expect("go http.NewRequest regex is valid"),
                method_group: Some("method"),
                implied_method: "GET",
                url_group: "url",
            },
        ],
    }
}

fn build_java_patterns() -> LangPatterns {
    LangPatterns {
        language: "java",
        matchers: vec![
            // HttpClient / RestTemplate / WebClient call sites — extract URL from
            // the first string argument, treat as generic HTTP call.
            LangMatcher {
                re: Regex::new(
                    r#"(?:HttpClient|RestTemplate|WebClient)[^.(]*\.\s*(?P<method>get|post|put|delete|getForObject|postForEntity|exchange|retrieve)\s*\([^)]*"(?P<url>[^"]+)""#,
                )
                .expect("java http client regex is valid"),
                method_group: Some("method"),
                implied_method: "GET",
                url_group: "url",
            },
        ],
    }
}

fn build_ruby_patterns() -> LangPatterns {
    LangPatterns {
        language: "ruby",
        matchers: vec![
            // Net::HTTP.get("url"), Faraday.get("url"), HTTParty.post("url"), etc.
            LangMatcher {
                re: Regex::new(
                    r#"(?:Net::HTTP|Faraday|HTTParty)\s*\.\s*(?P<method>get|post|put|delete|patch)\s*\(\s*(?:"(?P<url1>[^"]+)"|'(?P<url2>[^']+)')"#,
                )
                .expect("ruby http regex is valid"),
                method_group: Some("method"),
                implied_method: "GET",
                url_group: "url1",
            },
        ],
    }
}

fn build_csharp_patterns() -> LangPatterns {
    LangPatterns {
        language: "csharp",
        matchers: vec![
            // HttpClient.GetAsync("url"), PostAsync, PutAsync, DeleteAsync
            LangMatcher {
                re: Regex::new(
                    r#"HttpClient\s*\.\s*(?P<method>Get|Post|Put|Delete|Patch)Async\s*\(\s*(?:"(?P<url1>[^"]+)"|@?"(?P<url2>[^"]+)")"#,
                )
                .expect("csharp httpclient regex is valid"),
                method_group: Some("method"),
                implied_method: "GET",
                url_group: "url1",
            },
        ],
    }
}

/// Extract the URL from a capture that may have multiple url group variants
/// (url1 / url2 / url3 naming convention used across this module).
fn extract_url_multilang<'a>(cap: &'a regex::Captures<'_>, url_group: &str) -> Option<String> {
    // Try url1, url2 (for patterns with alternating quote groups), then the
    // bare url_group name (for patterns with a single capture group).
    cap.name("url1")
        .or_else(|| cap.name("url2"))
        .or_else(|| cap.name(url_group))
        .map(|m| m.as_str().to_string())
}

/// Normalise an HTTP method extracted from source to uppercase.
/// Java methods like `getForObject` map to GET, `postForEntity` to POST.
fn normalise_method(raw: &str) -> String {
    match raw.to_lowercase().as_str() {
        "get" | "getforobject" | "getforlist" | "retrieve" => "GET".into(),
        "post" | "postforentity" | "postforobject" => "POST".into(),
        "put" => "PUT".into(),
        "delete" => "DELETE".into(),
        "patch" => "PATCH".into(),
        "head" => "HEAD".into(),
        "exchange" => "GET".into(), // conservative default for exchange()
        other => other.to_uppercase(),
    }
}

/// Detect HTTP calls across all supported languages.
///
/// Calls the existing `detect_http_calls` for TS/JS, then adds detection for
/// Python, Go, Java, Ruby, and C#.  Returns a combined list of
/// `DetectedHttpCall` values ready for `match_http_calls_to_routes`.
pub fn detect_http_calls_all_languages(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<DetectedHttpCall>> {
    // Start with the existing TS/JS detection.
    let mut all_calls = detect_http_calls(conn, project_root)?;

    // Build language-specific pattern sets.
    let lang_patterns: Vec<LangPatterns> = vec![
        build_python_patterns(),
        build_go_patterns(),
        build_java_patterns(),
        build_ruby_patterns(),
        build_csharp_patterns(),
    ];

    for lang_set in &lang_patterns {
        let lang_filter = lang_set.language;

        let mut stmt = conn
            .prepare(
                "SELECT id, path FROM files WHERE language = ?1",
            )
            .context("Failed to prepare language file query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([lang_filter], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to query language files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect language file rows")?;

        for (file_id, rel_path) in files {
            let abs_path = project_root.join(&rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(e) => {
                    debug!(
                        path = %abs_path.display(),
                        err = %e,
                        "Skipping unreadable {} file",
                        lang_filter
                    );
                    continue;
                }
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                for matcher in &lang_set.matchers {
                    for cap in matcher.re.captures_iter(line_text) {
                        let Some(raw_url) = extract_url_multilang(&cap, matcher.url_group) else {
                            continue;
                        };

                        let method = if let Some(group) = matcher.method_group {
                            if let Some(m) = cap.name(group) {
                                normalise_method(m.as_str())
                            } else {
                                matcher.implied_method.to_string()
                            }
                        } else {
                            matcher.implied_method.to_string()
                        };

                        let url_pattern = normalise_url_pattern(&raw_url);

                        all_calls.push(DetectedHttpCall {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            http_method: method,
                            url_pattern,
                            raw_url,
                        });
                    }
                }
            }
        }
    }

    Ok(all_calls)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "frontend_http_tests.rs"]
mod tests;
