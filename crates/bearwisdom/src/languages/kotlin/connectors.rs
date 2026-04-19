// =============================================================================
// languages/kotlin/connectors.rs — Kotlin gRPC, MQ, REST connectors
//
// - KotlinGrpcConnector: needs symbol + inheritance joins → stays legacy.
// - KotlinMqConnector: flattened; emission lives in `extract_kotlin_mq_src`.
// - KotlinRestConnector: starts flattened into `extract_kotlin_rest_starts_src`;
//   stops still come from the `routes` table populated during parse.
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
// Plugin-facing composer
// ===========================================================================

pub fn extract_kotlin_connection_points(
    source: &str,
    file_path: &str,
) -> Vec<AbstractPoint> {
    let mut out = Vec::new();
    extract_kotlin_rest_starts_src(source, file_path, &mut out);
    extract_kotlin_mq_src(source, &mut out);
    out
}

// ===========================================================================
// KotlinGrpcConnector — inheritance + method-join; stays on DB path
// ===========================================================================

pub struct KotlinGrpcConnector;

impl Connector for KotlinGrpcConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "kotlin_grpc_stops",
            protocols: &[Protocol::Grpc],
            languages: &["kotlin"],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        true
    }

    fn extract(&self, conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Find Kotlin classes that extend *CoroutineImplBase or *ImplBase.
        let mut stmt = conn
            .prepare(
                "SELECT s.name, s.file_id
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.language = 'kotlin'
                   AND s.kind = 'class'
                   AND EXISTS (
                       SELECT 1 FROM edges e
                       JOIN symbols tgt ON tgt.id = e.target_id
                       WHERE e.source_id = s.id
                         AND e.kind = 'inherits'
                         AND (tgt.name LIKE '%CoroutineImplBase' OR tgt.name LIKE '%ImplBase')
                   )",
            )
            .context("Failed to prepare Kotlin gRPC class query")?;

        let classes: Vec<(String, i64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .context("Failed to query Kotlin gRPC classes")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Kotlin gRPC class rows")?;

        let mut points = Vec::new();

        for (_class_name, file_id) in &classes {
            let parent_name: Option<String> = conn
                .query_row(
                    "SELECT tgt.name FROM edges e
                     JOIN symbols src ON src.id = e.source_id
                     JOIN symbols tgt ON tgt.id = e.target_id
                     WHERE src.file_id = ?1
                       AND e.kind = 'inherits'
                       AND (tgt.name LIKE '%CoroutineImplBase' OR tgt.name LIKE '%ImplBase')
                     LIMIT 1",
                    rusqlite::params![file_id],
                    |row| row.get::<_, String>(0),
                )
                .ok();

            let service_name = parent_name
                .as_deref()
                .and_then(|n| {
                    n.strip_suffix("CoroutineImplBase")
                        .or_else(|| n.strip_suffix("ImplBase"))
                })
                .unwrap_or("")
                .to_string();

            if service_name.is_empty() {
                continue;
            }

            let mut method_stmt = conn
                .prepare(
                    "SELECT s.id, s.name, s.line
                     FROM symbols s
                     WHERE s.file_id = ?1 AND s.kind = 'method'",
                )
                .context("Failed to prepare Kotlin gRPC method query")?;

            let methods: Vec<(i64, String, u32)> = method_stmt
                .query_map(rusqlite::params![file_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                })
                .context("Failed to query Kotlin gRPC methods")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("Failed to collect Kotlin gRPC method rows")?;

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
                    framework: "grpc_kotlin".to_string(),
                    metadata: None,
                });
            }
        }

        Ok(points)
    }
}

// ===========================================================================
// KotlinMqConnector — flattened into extract_kotlin_mq_src
// ===========================================================================

pub struct KotlinMqConnector;

impl Connector for KotlinMqConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "kotlin_mq",
            protocols: &[Protocol::MessageQueue],
            languages: &["kotlin"],
        }
    }

    fn detect(&self, ctx: &ProjectContext) -> bool {
        ctx.has_dependency(ManifestKind::Maven, "org.springframework.kafka")
            || ctx.has_dependency(ManifestKind::Gradle, "org.springframework.kafka")
            || ctx.has_dependency(ManifestKind::Maven, "org.springframework.amqp")
            || ctx.has_dependency(ManifestKind::Gradle, "org.springframework.amqp")
            || ctx.has_dependency(ManifestKind::Maven, "org.apache.kafka")
            || ctx.has_dependency(ManifestKind::Gradle, "org.apache.kafka")
    }

    fn extract(&self, _conn: &Connection, _project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        Ok(Vec::new())
    }
}

pub fn extract_kotlin_mq_src(source: &str, out: &mut Vec<AbstractPoint>) {
    if !source.contains("kafkaTemplate")
        && !source.contains("rabbitTemplate")
        && !source.contains("@KafkaListener")
        && !source.contains("@RabbitListener")
    {
        return;
    }

    let re_kafka_send = regex::Regex::new(
        r#"kafkaTemplate\.send\s*\(\s*['"]([^'"]+)['"]"#,
    )
    .expect("kotlin kafka send regex");
    let re_rabbit_send = regex::Regex::new(
        r#"rabbitTemplate\.(?:convertAndSend|send)\s*\(\s*['"]([^'"]+)['"]"#,
    )
    .expect("kotlin rabbit send regex");
    // Kotlin uses arrayOf("topic") or ["topic"] syntax in annotations.
    let re_kafka_listener = regex::Regex::new(
        r#"@KafkaListener\s*\([^)]*topics\s*=\s*(?:\[[^\]]*['"]([^'"]+)['"]|['"]([^'"]+)['"])"#,
    )
    .expect("kotlin kafka listener regex");
    let re_rabbit_listener = regex::Regex::new(
        r#"@RabbitListener\s*\([^)]*queues\s*=\s*(?:\[[^\]]*['"]([^'"]+)['"]|['"]([^'"]+)['"])"#,
    )
    .expect("kotlin rabbit listener regex");

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

        for cap in re_kafka_send.captures_iter(line_text) {
            push(out, ConnectionRole::Start, cap[1].to_string(), line_no, "kafka");
        }
        for cap in re_rabbit_send.captures_iter(line_text) {
            push(out, ConnectionRole::Start, cap[1].to_string(), line_no, "rabbitmq");
        }
        for cap in re_kafka_listener.captures_iter(line_text) {
            if let Some(t) = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str().to_string()) {
                push(out, ConnectionRole::Stop, t, line_no, "kafka");
            }
        }
        for cap in re_rabbit_listener.captures_iter(line_text) {
            if let Some(q) = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str().to_string()) {
                push(out, ConnectionRole::Stop, q, line_no, "rabbitmq");
            }
        }
    }
}

// ===========================================================================
// KotlinRestConnector — starts flattened, stops on DB
// ===========================================================================

pub struct KotlinRestConnector;

impl Connector for KotlinRestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "kotlin_rest",
            protocols: &[Protocol::Rest],
            languages: &["kotlin"],
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
        let mut points = Vec::new();
        extract_kotlin_rest_stops(conn, &mut points)?;
        Ok(points)
    }
}

fn extract_kotlin_rest_stops(conn: &Connection, out: &mut Vec<ConnectionPoint>) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             JOIN files f ON f.id = r.file_id
             WHERE f.language = 'kotlin'
               AND r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare Kotlin REST stops query")?;

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
        .context("Failed to query Kotlin routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read Kotlin route row")?;
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

/// Kotlin REST client-call starts: Retrofit `@GET("/path")`, OkHttp `.url("/x")`,
/// Ktor `client.get("/x")`.
pub fn extract_kotlin_rest_starts_src(
    source: &str,
    file_path: &str,
    out: &mut Vec<AbstractPoint>,
) {
    if kotlin_rest_is_test_file(file_path) {
        return;
    }
    if !source.contains("@GET")
        && !source.contains("@POST")
        && !source.contains("@PUT")
        && !source.contains("@DELETE")
        && !source.contains("@PATCH")
        && !source.contains("@HEAD")
        && !source.contains(".url")
        && !source.contains("client")
        && !source.contains("httpClient")
    {
        return;
    }

    let re_retrofit = regex::Regex::new(
        r#"@(?P<method>GET|POST|PUT|DELETE|PATCH|HEAD)\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("kotlin retrofit annotation regex");
    let re_okhttp = regex::Regex::new(r#"\.url\s*\(\s*"(?P<url>[^"]+)"\s*\)"#)
        .expect("kotlin okhttp url regex");
    let re_ktor = regex::Regex::new(
        r#"(?:client|httpClient)\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*(?:<[^>]*>)?\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("kotlin ktor client regex");

    let push = |out: &mut Vec<AbstractPoint>,
                key: String,
                line: u32,
                method: String,
                framework: &str| {
        let mut meta = HashMap::new();
        meta.insert("method".to_string(), method);
        meta.insert("framework".to_string(), framework.to_string());
        out.push(AbstractPoint {
            kind: ConnectionKind::Rest,
            role: ConnectionRole::Start,
            key,
            line,
            col: 1,
            symbol_qname: String::new(),
            meta,
        });
    };

    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        for cap in re_retrofit.captures_iter(line_text) {
            let raw_url = cap["url"].to_string();
            if !kotlin_rest_looks_like_api_url(&raw_url) {
                continue;
            }
            push(
                out,
                rest_normalise_url_pattern(&raw_url),
                line_no,
                cap["method"].to_string(),
                "retrofit",
            );
        }

        if let Some(cap) = re_okhttp.captures(line_text) {
            let raw_url = cap["url"].to_string();
            if kotlin_rest_looks_like_api_url(&raw_url) {
                push(
                    out,
                    rest_normalise_url_pattern(&raw_url),
                    line_no,
                    "GET".to_string(),
                    "okhttp",
                );
            }
        }

        for cap in re_ktor.captures_iter(line_text) {
            let raw_url = cap["url"].to_string();
            if !kotlin_rest_looks_like_api_url(&raw_url) {
                continue;
            }
            let method = cap
                .name("method")
                .map(|m| m.as_str().to_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            push(
                out,
                rest_normalise_url_pattern(&raw_url),
                line_no,
                method,
                "ktor",
            );
        }
    }
}

fn kotlin_rest_is_test_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    lower.contains("test") || lower.contains("spec")
}

fn kotlin_rest_looks_like_api_url(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() {
            return false;
        }
        return kotlin_rest_looks_like_api_url(path);
    }
    s.starts_with('/')
        || s.contains("/api/")
        || s.contains("/v1/")
        || s.contains("/v2/")
        || s.contains("/v3/")
        || s.contains("/{")
}

fn rest_normalise_url_pattern(raw: &str) -> String {
    let without_query = raw.split('?').next().unwrap_or(raw);
    let re_tmpl = regex::Regex::new(r"\$\{[^}]+\}").expect("template regex");
    re_tmpl.replace_all(without_query, "{param}").into_owned()
}

#[cfg(test)]
mod plugin_source_scan_tests {
    use super::*;

    #[test]
    fn kotlin_rest_retrofit_get() {
        let src = r#"@GET("/users/{id}") fun getUser(@Path("id") id: Long): User"#;
        let mut out = Vec::new();
        extract_kotlin_rest_starts_src(src, "src/main/kotlin/Api.kt", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "/users/{id}");
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("GET"));
        assert_eq!(out[0].meta.get("framework").map(String::as_str), Some("retrofit"));
    }

    #[test]
    fn kotlin_rest_ktor_post() {
        let src = r#"val r = client.post("/api/users") { body = u }"#;
        let mut out = Vec::new();
        extract_kotlin_rest_starts_src(src, "src/main/kotlin/App.kt", &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].meta.get("method").map(String::as_str), Some("POST"));
        assert_eq!(out[0].meta.get("framework").map(String::as_str), Some("ktor"));
    }

    #[test]
    fn kotlin_mq_kafka_listener_stop() {
        let src = r#"@KafkaListener(topics = ["orders"])\nfun onOrder(m: Msg) {}"#;
        let mut out = Vec::new();
        extract_kotlin_mq_src(src, &mut out);
        let stops: Vec<_> = out.iter().filter(|p| p.role == ConnectionRole::Stop).collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].key, "orders");
    }

    #[test]
    fn kotlin_mq_kafka_send_start() {
        let src = r#"kafkaTemplate.send("users", msg)"#;
        let mut out = Vec::new();
        extract_kotlin_mq_src(src, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "users");
        assert_eq!(out[0].role, ConnectionRole::Start);
    }

    #[test]
    fn composer_combines() {
        let src = r#"
@GET("/api/x") fun x()
kafkaTemplate.send("y", m)
"#;
        let points = extract_kotlin_connection_points(src, "App.kt");
        let has = |k: ConnectionKind| points.iter().any(|p| p.kind == k);
        assert!(has(ConnectionKind::Rest));
        assert!(has(ConnectionKind::MessageQueue));
    }
}
