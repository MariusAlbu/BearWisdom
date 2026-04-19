// =============================================================================
// languages/rust_lang/connectors.rs — Rust language connectors
//
// Flattened pattern (see CONNECTOR_MIGRATION.md):
//   - Source-scan halves live in `extract_rust_connection_points(source,
//     file_path)`, wired into `RustLangPlugin::extract_connection_points`.
//     They emit abstract `crate::types::ConnectionPoint` values at parse
//     time.
//   - Legacy `Connector::extract` impls stay registered for their
//     `detect(ctx)` gating + any DB-table work that still lives post-parse
//     (REST stops from `routes`, gRPC stop detection needs the symbol table).
// =============================================================================

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::types::{
    ConnectionKind, ConnectionPoint as AbstractPoint, ConnectionRole,
};

// ===========================================================================
// Plugin-facing entry point
// ===========================================================================

/// Invoked from `RustLangPlugin::extract_connection_points`. Returns all
/// source-scannable ConnectionPoints for one `.rs` file.
pub fn extract_rust_connection_points(
    source: &str,
    file_path: &str,
) -> Vec<AbstractPoint> {
    let mut out = Vec::new();
    extract_tauri_ipc_rust_src(source, &mut out);
    extract_rust_rest_starts_src(source, file_path, &mut out);
    extract_rust_mq_src(source, &mut out);
    out
}

// ===========================================================================
// Tauri IPC — Rust handler side (Stop for #[command], Start for .emit())
// ===========================================================================

pub struct TauriIpcConnector;

impl Connector for TauriIpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "tauri_ipc_rust",
            protocols: &[Protocol::Ipc],
            languages: &["rust"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.has_dependency(ManifestKind::Cargo, "tauri")
    }

    fn extract(
        &self,
        _conn: &Connection,
        _project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        // Flattened: `extract_tauri_ipc_rust_src` emits both
        // #[tauri::command] Stop points and app.emit()/window.emit() Start
        // points during parse.
        Ok(Vec::new())
    }
}

/// Scan one Rust file for Tauri IPC markers.
///
///   - `#[tauri::command]` or `#[command]` on the line before a fn decl →
///     Stop point with key = function name.
///   - `.emit("name", ...)` / `.emit('name', ...)` → Start point with key =
///     event name.
pub fn extract_tauri_ipc_rust_src(source: &str, out: &mut Vec<AbstractPoint>) {
    if !source.contains("command") && !source.contains(".emit") {
        return;
    }

    let re_attr = regex::Regex::new(r"#\[(?:tauri::)?command\]")
        .expect("command attr regex is valid");
    let re_fn = regex::Regex::new(r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*[(<]")
        .expect("fn decl regex is valid");
    let re_emit = regex::Regex::new(
        r#"\.emit\s*\(\s*(?:"(?P<name1>[^"]+)"|'(?P<name2>[^']+)')"#,
    )
    .expect("emit regex");

    let mut next_line_is_command = false;
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        if re_attr.is_match(line_text) {
            next_line_is_command = true;
            continue;
        }

        if next_line_is_command {
            next_line_is_command = false;
            if let Some(cap) = re_fn.captures(line_text) {
                let cmd_name = cap[1].to_string();
                let mut meta = HashMap::new();
                meta.insert("framework".to_string(), "tauri".to_string());
                out.push(AbstractPoint {
                    kind: ConnectionKind::Ipc,
                    role: ConnectionRole::Stop,
                    key: cmd_name.clone(),
                    line: line_no,
                    col: 1,
                    symbol_qname: cmd_name,
                    meta,
                });
            }
        }

        for cap in re_emit.captures_iter(line_text) {
            let name = cap
                .name("name1")
                .or_else(|| cap.name("name2"))
                .map(|m| m.as_str().to_string());
            if let Some(key) = name {
                let mut meta = HashMap::new();
                meta.insert("framework".to_string(), "tauri".to_string());
                out.push(AbstractPoint {
                    kind: ConnectionKind::Ipc,
                    role: ConnectionRole::Start,
                    key,
                    line: line_no,
                    col: 1,
                    symbol_qname: String::new(),
                    meta,
                });
            }
        }
    }
}

// ===========================================================================
// Rust REST — starts (client calls) scan, stops (routes table) DB
// ===========================================================================

pub struct RustRestConnector;

impl Connector for RustRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "rust_rest",
            protocols: &[Protocol::Rest],
            languages: &["rust"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(
        &self,
        conn: &Connection,
        _project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        // Starts (reqwest calls) migrated into `extract_rust_rest_starts_src`.
        // Stops still read from the `routes` table populated by the Rust
        // parser plugin.
        let mut points = Vec::new();
        extract_rust_rest_stops(conn, &mut points)?;
        Ok(points)
    }
}

fn extract_rust_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'rust'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare Rust REST stops query")?;

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
        .context("Failed to query Rust routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read Rust route row")?;
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

/// Source-scan reqwest client calls; emit Start points for each.
pub fn extract_rust_rest_starts_src(
    source: &str,
    file_path: &str,
    out: &mut Vec<AbstractPoint>,
) {
    if rust_rest_is_test_file(file_path) {
        return;
    }
    if !source.contains("reqwest") && !source.contains("client") {
        return;
    }

    let re_reqwest = regex::Regex::new(
        r#"(?:reqwest(?:::\w+)?|client)\s*(?:::\w+)?\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("rust reqwest regex");
    let re_reqwest_free = regex::Regex::new(
        r#"reqwest\s*::\s*get\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("rust reqwest free fn regex");

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        for cap in re_reqwest.captures_iter(line_text) {
            let raw_url = cap["url"].to_string();
            if !rust_rest_looks_like_api_url(&raw_url) {
                continue;
            }
            let method = cap
                .name("method")
                .map(|m| m.as_str().to_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            let url_pattern = rest_normalise_url_pattern_rust(&raw_url);
            let mut meta = HashMap::new();
            meta.insert("method".to_string(), method);
            meta.insert("framework".to_string(), "reqwest".to_string());
            out.push(AbstractPoint {
                kind: ConnectionKind::Rest,
                role: ConnectionRole::Start,
                key: url_pattern,
                line: line_no,
                col: 1,
                symbol_qname: String::new(),
                meta,
            });
        }

        for cap in re_reqwest_free.captures_iter(line_text) {
            let raw_url = cap["url"].to_string();
            if !rust_rest_looks_like_api_url(&raw_url) {
                continue;
            }
            let url_pattern = rest_normalise_url_pattern_rust(&raw_url);
            let mut meta = HashMap::new();
            meta.insert("method".to_string(), "GET".to_string());
            meta.insert("framework".to_string(), "reqwest".to_string());
            out.push(AbstractPoint {
                kind: ConnectionKind::Rest,
                role: ConnectionRole::Start,
                key: url_pattern,
                line: line_no,
                col: 1,
                symbol_qname: String::new(),
                meta,
            });
        }
    }
}

fn rust_rest_is_test_file(rel_path: &str) -> bool {
    let lower = rel_path.replace('\\', "/").to_lowercase();
    lower.contains("/tests/")
        || lower.starts_with("tests/")
        || lower.contains("_test.rs")
        || lower.contains("/benches/")
        || lower.starts_with("benches/")
}

fn rust_rest_looks_like_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() {
            return false;
        }
        return rust_rest_looks_like_api_url(path);
    }
    s.starts_with('/')
        || s.contains("/api/")
        || s.contains("/v1/")
        || s.contains("/v2/")
        || s.contains("/v3/")
        || s.contains("/{")
}

fn rest_normalise_url_pattern_rust(raw: &str) -> String {
    raw.split('?').next().unwrap_or(raw).to_string()
}

// ===========================================================================
// Rust gRPC — tonic `impl XxxServer for` detection (DB symbol join, legacy)
// ===========================================================================

pub struct RustGrpcConnector;

impl Connector for RustGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "rust_grpc_stops",
            protocols: &[Protocol::Grpc],
            languages: &["rust"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.has_dependency(ManifestKind::Cargo, "tonic")
            || ctx.has_dependency(ManifestKind::Cargo, "grpcio")
            || ctx.has_dependency(ManifestKind::Cargo, "prost")
    }

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        let re_impl = regex::Regex::new(r#"impl\s+(\w+Server)\s+for\s+(\w+)"#)
            .expect("rust tonic impl regex");

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'rust'")
            .context("Failed to prepare Rust files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to query Rust files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Rust file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in &files {
            let abs_path = project_root.join(rel_path);
            let source = match std::fs::read_to_string(&abs_path) {
                Ok(s) => s,
                Err(_) => continue,
            };

            for (line_idx, line_text) in source.lines().enumerate() {
                let line_no = (line_idx + 1) as u32;

                if let Some(cap) = re_impl.captures(line_text) {
                    let trait_name = &cap[1];
                    let service_name = trait_name
                        .strip_suffix("Server")
                        .unwrap_or(trait_name)
                        .to_string();

                    if service_name.is_empty() {
                        continue;
                    }

                    let mut method_stmt = conn
                        .prepare(
                            "SELECT s.id, s.name, s.line
                             FROM symbols s
                             WHERE s.file_id = ?1 AND s.kind = 'method'
                               AND s.line >= ?2",
                        )
                        .context("Failed to prepare Rust gRPC method query")?;

                    let methods: Vec<(i64, String, u32)> = method_stmt
                        .query_map(rusqlite::params![file_id, line_no], |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, u32>(2)?,
                            ))
                        })
                        .context("Failed to query Rust gRPC methods")?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .context("Failed to collect Rust gRPC method rows")?;

                    for (sym_id, method_name, line) in methods {
                        let key = format!("{service_name}.{method_name}");
                        points.push(ConnectionPoint {
                            file_id: *file_id,
                            symbol_id: Some(sym_id),
                            line,
                            protocol: Protocol::Grpc,
                            direction: FlowDirection::Stop,
                            key,
                            method: String::new(),
                            framework: "tonic".to_string(),
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
// Rust MQ — rdkafka / lapin / async-nats source scan
// ===========================================================================

pub struct RustMqConnector;

impl Connector for RustMqConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "rust_mq",
            protocols: &[Protocol::MessageQueue],
            languages: &["rust"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.has_dependency(ManifestKind::Cargo, "rdkafka")
            || ctx.has_dependency(ManifestKind::Cargo, "kafka")
            || ctx.has_dependency(ManifestKind::Cargo, "lapin")
            || ctx.has_dependency(ManifestKind::Cargo, "async-nats")
            || ctx.has_dependency(ManifestKind::Cargo, "nats")
    }

    fn extract(&self, _conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Flattened: see `extract_rust_mq_src`.
        Ok(Vec::new())
    }
}

/// Source-scan Rust MQ client calls:
///   - rdkafka: `FutureRecord::to("topic")` / `BaseRecord::to("topic")` (produce)
///              `consumer.subscribe(&["topic"])` (consume)
///   - lapin:   `basic_publish("exchange", "routing_key")` (produce)
///              `basic_consume("queue")` (consume)
///   - nats:    `.publish("subject")` (produce) / `.subscribe("subject")` (consume)
pub fn extract_rust_mq_src(source: &str, out: &mut Vec<AbstractPoint>) {
    if !source.contains("FutureRecord")
        && !source.contains("BaseRecord")
        && !source.contains("basic_publish")
        && !source.contains("basic_consume")
        && !source.contains(".subscribe")
        && !source.contains(".publish")
    {
        return;
    }

    let re_rdkafka_send = regex::Regex::new(
        r#"(?:FutureRecord|BaseRecord)::to\s*\(\s*['"`]([^'"`]+)['"`]"#,
    )
    .expect("rust rdkafka send regex");
    let re_rdkafka_subscribe = regex::Regex::new(
        r#"\.subscribe\s*\(\s*&\s*\[\s*['"`]([^'"`]+)['"`]"#,
    )
    .expect("rust rdkafka subscribe regex");
    let re_lapin_publish = regex::Regex::new(
        r#"basic_publish\s*\(\s*['"`]([^'"`]+)['"`]\s*,\s*['"`]([^'"`]+)['"`]"#,
    )
    .expect("rust lapin publish regex");
    let re_lapin_consume = regex::Regex::new(
        r#"basic_consume\s*\(\s*['"`]([^'"`]+)['"`]"#,
    )
    .expect("rust lapin consume regex");
    let re_nats_subscribe = regex::Regex::new(
        r#"\.subscribe\s*\(\s*['"`]([^'"`]+)['"`]"#,
    )
    .expect("rust nats subscribe regex");
    let re_nats_publish = regex::Regex::new(
        r#"\.publish\s*\(\s*['"`]([^'"`]+)['"`]"#,
    )
    .expect("rust nats publish regex");

    let push = |out: &mut Vec<AbstractPoint>,
                role: ConnectionRole,
                key: String,
                line: u32,
                framework: &str| {
        let mut meta = HashMap::new();
        meta.insert("framework".to_string(), framework.to_string());
        out.push(AbstractPoint {
            kind: ConnectionKind::MessageQueue,
            role,
            key,
            line,
            col: 1,
            symbol_qname: String::new(),
            meta,
        });
    };

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        for cap in re_rdkafka_send.captures_iter(line_text) {
            push(out, ConnectionRole::Start, cap[1].to_string(), line_no, "kafka");
        }
        for cap in re_rdkafka_subscribe.captures_iter(line_text) {
            push(out, ConnectionRole::Stop, cap[1].to_string(), line_no, "kafka");
        }
        for cap in re_lapin_publish.captures_iter(line_text) {
            push(out, ConnectionRole::Start, cap[2].to_string(), line_no, "rabbitmq");
        }
        for cap in re_lapin_consume.captures_iter(line_text) {
            push(out, ConnectionRole::Stop, cap[1].to_string(), line_no, "rabbitmq");
        }
        // NATS subscribe / publish regexes overlap with the rdkafka subscribe
        // + the lapin nothing; the frameworks are mutually exclusive by
        // dependency, but we guard on both to avoid double-emitting in a
        // codebase that uses both (rare).
        if !re_rdkafka_subscribe.is_match(line_text) {
            for cap in re_nats_subscribe.captures_iter(line_text) {
                push(out, ConnectionRole::Stop, cap[1].to_string(), line_no, "nats");
            }
        }
        for cap in re_nats_publish.captures_iter(line_text) {
            push(out, ConnectionRole::Start, cap[1].to_string(), line_no, "nats");
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tauri_command_attr_produces_stop() {
        let src = "#[tauri::command]\npub async fn greet(name: String) -> String {\n    format!(\"Hi {}\", name)\n}\n";
        let mut out = Vec::new();
        extract_tauri_ipc_rust_src(src, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "greet");
        assert_eq!(out[0].role, ConnectionRole::Stop);
        assert_eq!(out[0].kind, ConnectionKind::Ipc);
        assert_eq!(out[0].line, 2, "fn decl is on line 2");
        assert_eq!(out[0].meta.get("framework").map(String::as_str), Some("tauri"));
    }

    #[test]
    fn tauri_short_command_attr_matches() {
        let src = "#[command]\nfn close_splashscreen(window: Window) {}\n";
        let mut out = Vec::new();
        extract_tauri_ipc_rust_src(src, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "close_splashscreen");
    }

    #[test]
    fn tauri_emit_produces_start() {
        let src = "fn notify(app: &AppHandle) {\n    app.emit(\"progress\", &payload).unwrap();\n}\n";
        let mut out = Vec::new();
        extract_tauri_ipc_rust_src(src, &mut out);
        let starts: Vec<_> = out.iter().filter(|p| p.role == ConnectionRole::Start).collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].key, "progress");
    }

    #[test]
    fn rust_rest_starts_detects_reqwest_get() {
        let src = "async fn call() {\n    let _ = reqwest::get(\"/api/users\").await;\n}\n";
        let mut out = Vec::new();
        extract_rust_rest_starts_src(src, "src/lib.rs", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "/api/users");
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("GET"));
        assert_eq!(out[0].role, ConnectionRole::Start);
    }

    #[test]
    fn rust_rest_starts_skips_test_files() {
        let src = "reqwest::get(\"/api/x\").await;";
        let mut out = Vec::new();
        extract_rust_rest_starts_src(src, "tests/integration.rs", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn rust_rest_starts_filters_non_api_urls() {
        // Bare domain (no path) → filtered.
        let src = "reqwest::get(\"https://example.com\").await;";
        let mut out = Vec::new();
        extract_rust_rest_starts_src(src, "src/lib.rs", &mut out);
        assert!(out.is_empty(), "bare domain without path is filtered");
    }

    #[test]
    fn rust_mq_kafka_send_start() {
        let src = "let record = FutureRecord::to(\"orders\").payload(&data);";
        let mut out = Vec::new();
        extract_rust_mq_src(src, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "orders");
        assert_eq!(out[0].role, ConnectionRole::Start);
        assert_eq!(out[0].meta.get("framework").map(String::as_str), Some("kafka"));
    }

    #[test]
    fn rust_mq_lapin_publish_uses_routing_key() {
        let src = "channel.basic_publish(\"my_exchange\", \"route.key\", opts, body).await;";
        let mut out = Vec::new();
        extract_rust_mq_src(src, &mut out);
        let starts: Vec<_> = out.iter().filter(|p| p.role == ConnectionRole::Start).collect();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].key, "route.key");
        assert_eq!(starts[0].meta.get("framework").map(String::as_str), Some("rabbitmq"));
    }

    #[test]
    fn rust_mq_no_markers_is_empty() {
        let mut out = Vec::new();
        extract_rust_mq_src("fn main() { println!(\"hi\"); }", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn extract_rust_connection_points_composes_all_three() {
        let src = r#"
#[tauri::command]
fn cmd(name: String) -> String { name }

async fn client() {
    let _ = reqwest::get("/api/ping").await;
}

fn mq() {
    let r = FutureRecord::to("topic");
}
"#;
        let points = extract_rust_connection_points(src, "src/lib.rs");
        let ipc_count = points.iter().filter(|p| p.kind == ConnectionKind::Ipc).count();
        let rest_count = points.iter().filter(|p| p.kind == ConnectionKind::Rest).count();
        let mq_count = points.iter().filter(|p| p.kind == ConnectionKind::MessageQueue).count();
        assert_eq!(ipc_count, 1);
        assert_eq!(rest_count, 1);
        assert_eq!(mq_count, 1);
    }
}
