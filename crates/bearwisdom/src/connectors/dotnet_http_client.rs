// =============================================================================
// connectors/dotnet_http_client.rs  —  .NET HTTP client connector
//
// Detects HTTP API calls in .NET client code (MAUI, Blazor, console apps)
// that use abstraction layers like IRequestProvider, typed HttpClient
// wrappers, or string-based URL construction with const ApiUrlBase patterns.
//
// Detection strategy:
//   1. Find const/static string fields containing API path segments
//      (e.g., `const string ApiUrlBase = "api/catalog"`)
//   2. Find method calls that reference those paths in string interpolation
//      (e.g., `_requestProvider.GetAsync<T>($"{ApiUrlBase}/items/{id}")`)
//   3. Extract the HTTP method from the wrapper method name (GetAsync → GET)
//   4. Match extracted URL patterns against the routes table
//   5. Insert flow_edges for matched pairs
//
// This complements frontend_http.rs which catches direct HttpClient.*Async
// calls with literal URLs.  This connector catches the abstraction patterns
// common in MAUI, Blazor, and typed HTTP client architectures.
// =============================================================================

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::connectors::http_api::{normalise_route, routes_match};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DotnetHttpCall {
    pub file_id: i64,
    pub line: u32,
    pub http_method: String,
    pub url_pattern: String,
    pub language: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detect .NET HTTP client calls and match them to backend routes.
///
/// Returns the number of flow_edges created.
pub fn connect(conn: &Connection, project_root: &std::path::Path) -> Result<u32> {
    let calls = detect_dotnet_http_calls(conn, project_root)?;
    if calls.is_empty() {
        return Ok(0);
    }

    let matched = match_calls_to_routes(conn, &calls)?;
    info!(
        "DotnetHttpClient: {} calls detected, {} matched to routes",
        calls.len(),
        matched
    );
    Ok(matched)
}

/// Scan all C# files for HTTP client call patterns.
pub fn detect_dotnet_http_calls(
    conn: &Connection,
    project_root: &std::path::Path,
) -> Result<Vec<DotnetHttpCall>> {
    // Load all C# files from the index.
    let mut stmt = conn.prepare(
        "SELECT id, path FROM files WHERE language = 'csharp'"
    ).context("Failed to query C# files")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    // Compile patterns once.
    let re_api_const = Regex::new(
        r#"(?:const|static\s+readonly)\s+string\s+\w*(?:Api|Url|Base|Endpoint)\w*\s*=\s*"([^"]+)""#,
    ).expect("api const regex is valid");

    // Match wrapper method calls: _provider.GetAsync, _httpClient.PostAsync, etc.
    let re_wrapper_call = Regex::new(
        r#"(?:_\w+|this\.\w+)\s*\.\s*(?P<method>Get|Post|Put|Delete|Patch)Async\s*(?:<[^>]*>)?\s*\(\s*(?:(?:"(?P<url>[^"]+)")|(?:\$"(?P<interp>[^"]+)"))"#,
    ).expect("wrapper call regex is valid");

    // Match UriHelper.CombineUri or similar patterns with interpolated strings.
    let re_uri_combine = Regex::new(
        r#"(?:UriHelper\.CombineUri|new\s+Uri)\s*\([^,]+,\s*(?:\$"(?P<interp>[^"]+)"|"(?P<literal>[^"]+)")"#,
    ).expect("uri combine regex is valid");

    let mut calls = Vec::new();

    for (file_id, rel_path) in &files {
        let abs_path = project_root.join(rel_path);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Step 1: Collect API URL base constants from this file.
        let mut api_bases: Vec<(String, String)> = Vec::new(); // (const_name_hint, value)
        for cap in re_api_const.captures_iter(&content) {
            if let Some(val) = cap.get(1) {
                api_bases.push((String::new(), val.as_str().to_string()));
            }
        }

        // Step 2: Find wrapper method calls.
        for (line_num, line) in content.lines().enumerate() {
            let line_1based = (line_num + 1) as u32;

            // Pattern A: _provider.GetAsync<T>("url") or _provider.GetAsync<T>($"url")
            if let Some(cap) = re_wrapper_call.captures(line) {
                let method = cap.name("method").map(|m| m.as_str()).unwrap_or("GET");
                let url = cap.name("url")
                    .or_else(|| cap.name("interp"))
                    .map(|m| m.as_str());

                if let Some(url) = url {
                    let normalised = normalise_interpolated_url(url, &api_bases);
                    calls.push(DotnetHttpCall {
                        file_id: *file_id,
                        line: line_1based,
                        http_method: method.to_uppercase(),
                        url_pattern: normalised,
                        language: "csharp".to_string(),
                    });
                    continue;
                }
            }

            // Pattern B: UriHelper.CombineUri(base, $"{ApiUrlBase}/items/{id}")
            if let Some(cap) = re_uri_combine.captures(line) {
                let url = cap.name("interp")
                    .or_else(|| cap.name("literal"))
                    .map(|m| m.as_str());

                if let Some(url) = url {
                    // Infer HTTP method from surrounding context.
                    let method = infer_method_from_context(&content, line_num);
                    let normalised = normalise_interpolated_url(url, &api_bases);
                    calls.push(DotnetHttpCall {
                        file_id: *file_id,
                        line: line_1based,
                        http_method: method,
                        url_pattern: normalised,
                        language: "csharp".to_string(),
                    });
                }
            }
        }
    }

    debug!("{} .NET HTTP client calls detected", calls.len());
    Ok(calls)
}

/// Match detected .NET HTTP calls to backend routes and insert flow_edges.
fn match_calls_to_routes(conn: &Connection, calls: &[DotnetHttpCall]) -> Result<u32> {
    // Load routes.
    let mut stmt = conn.prepare(
        "SELECT r.id, f.id, f.path, r.http_method, r.route_template,
                r.resolved_route, r.line, s.name
         FROM routes r
         JOIN files f ON r.file_id = f.id
         LEFT JOIN symbols s ON r.symbol_id = s.id"
    ).context("Failed to load routes")?;

    struct RouteRow {
        file_id: i64,
        http_method: String,
        route_template: String,
        resolved_route: Option<String>,
        line: u32,
        handler_name: Option<String>,
    }

    let routes: Vec<RouteRow> = stmt
        .query_map([], |r| {
            Ok(RouteRow {
                file_id: r.get::<_, i64>(1)?,
                http_method: r.get::<_, String>(3)?,
                route_template: r.get::<_, String>(4)?,
                resolved_route: r.get::<_, Option<String>>(5)?,
                line: r.get::<_, Option<u32>>(6)?.unwrap_or(0),
                handler_name: r.get::<_, Option<String>>(7)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    if routes.is_empty() {
        return Ok(0);
    }

    let mut created: u32 = 0;

    for call in calls {
        let call_norm = normalise_route(&call.url_pattern);

        for route in &routes {
            let method_ok = route.http_method.eq_ignore_ascii_case(&call.http_method);
            if !method_ok {
                continue;
            }

            let route_norm = normalise_route(
                route.resolved_route.as_deref().unwrap_or(&route.route_template),
            );

            if routes_match(&call_norm, &route_norm) {
                let confidence: f64 = if call_norm == route_norm { 0.90 } else { 0.75 };

                let result = conn.execute(
                    "INSERT OR IGNORE INTO flow_edges (
                        source_file_id, source_line, source_symbol, source_language,
                        target_file_id, target_line, target_symbol, target_language,
                        edge_type, protocol, http_method, url_pattern, confidence
                     ) VALUES (
                        ?1, ?2, NULL, ?3,
                        ?4, ?5, ?6,   'csharp',
                        'http_call', 'http', ?7, ?8, ?9
                     )",
                    rusqlite::params![
                        call.file_id,
                        call.line,
                        call.language,
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
                    Ok(_) => {}
                    Err(e) => {
                        debug!(err = %e, "Failed to insert flow_edge for dotnet http_call");
                    }
                }
            }
        }
    }

    Ok(created)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Normalise a C# interpolated string URL to a route pattern.
///
/// - Replaces `{variable}` interpolation expressions with `{*}`
/// - Strips query strings
/// - Resolves known ApiUrlBase constants inline
fn normalise_interpolated_url(url: &str, api_bases: &[(String, String)]) -> String {
    let mut result = url.to_string();

    // Inline known API base constants (e.g., {ApiUrlBase} → "api/catalog").
    for (_name, value) in api_bases {
        // Handle both {ApiUrlBase} and {SomeConst} patterns.
        // We don't know the exact const name, so replace any {Identifier}
        // that looks like a const reference (starts with uppercase or contains "Api/Url/Base").
        if !value.is_empty() {
            result = result.replace(&format!("{{{}}}", "ApiUrlBase"), value);
            result = result.replace(&format!("{{{}}}", "ApiUrl"), value);
        }
    }

    // Replace remaining C# interpolation expressions with {*}.
    let re_interp = Regex::new(r"\{[^}]+\}").expect("interp regex");
    let result = re_interp.replace_all(&result, "{*}").to_string();

    // Strip query string.
    let result = result.split('?').next().unwrap_or(&result).to_string();

    // Clean up double slashes.
    result.replace("//", "/")
}

/// Infer HTTP method by looking at surrounding lines for GetAsync/PostAsync calls.
fn infer_method_from_context(content: &str, target_line: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let start = target_line.saturating_sub(3);
    let end = (target_line + 3).min(lines.len());

    for i in start..end {
        let line = lines[i].to_lowercase();
        if line.contains("getasync") || line.contains(".get(") || line.contains("get<") {
            return "GET".to_string();
        }
        if line.contains("postasync") || line.contains(".post(") || line.contains("post<") {
            return "POST".to_string();
        }
        if line.contains("putasync") || line.contains(".put(") || line.contains("put<") {
            return "PUT".to_string();
        }
        if line.contains("deleteasync") || line.contains(".delete(") {
            return "DELETE".to_string();
        }
        if line.contains("patchasync") || line.contains(".patch(") {
            return "PATCH".to_string();
        }
    }

    "GET".to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "dotnet_http_client_tests.rs"]
mod tests;
