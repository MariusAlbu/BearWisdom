// =============================================================================
// languages/kotlin/connectors.rs — Kotlin gRPC and MQ connectors
//
// KotlinGrpcConnector:
//   Detects Kotlin gRPC service implementations.  Kotlin gRPC stubs follow the
//   same pattern as Java: generated base class is `{ServiceName}CoroutineImplBase`
//   (for coroutine stubs) or `{ServiceName}ImplBase` (for blocking stubs).
//
// KotlinMqConnector:
//   Detects Kotlin Spring Kafka / Spring AMQP patterns (same annotations as Java
//   since Kotlin interoperates with the Spring framework directly).
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::connectors::traits::{Connector, ConnectorDescriptor};
use crate::connectors::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;

// ===========================================================================
// KotlinGrpcConnector
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
            // Derive service name from the base class name.
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
// KotlinMqConnector
// ===========================================================================

/// Detects Kotlin Spring Kafka and Spring AMQP (RabbitMQ) patterns.
///
/// Kotlin projects use the same Spring annotations as Java projects because
/// Kotlin interoperates directly with the Spring framework.
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

    fn extract(&self, conn: &Connection, project_root: &Path) -> Result<Vec<ConnectionPoint>> {
        // Producers:
        //   kafkaTemplate.send("topic", ...)
        //   rabbitTemplate.convertAndSend("exchange", "key", ...)
        //
        // Consumers:
        //   @KafkaListener(topics = ["topic"])   — Kotlin array literal
        //   @KafkaListener(topics = ["t1", "t2"])
        //   @RabbitListener(queues = ["queue"])

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

        let mut stmt = conn
            .prepare("SELECT id, path FROM files WHERE language = 'kotlin'")
            .context("Failed to prepare Kotlin files query")?;

        let files: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .context("Failed to query Kotlin files")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect Kotlin file rows")?;

        let mut points = Vec::new();

        for (file_id, rel_path) in files {
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

                for cap in re_rabbit_send.captures_iter(line_text) {
                    points.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::MessageQueue,
                        direction: FlowDirection::Start,
                        key: cap[1].to_string(),
                        method: String::new(),
                        framework: "rabbitmq".to_string(),
                        metadata: None,
                    });
                }

                for cap in re_kafka_listener.captures_iter(line_text) {
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

                for cap in re_rabbit_listener.captures_iter(line_text) {
                    let queue = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str());
                    if let Some(q) = queue {
                        points.push(ConnectionPoint {
                            file_id,
                            symbol_id: None,
                            line: line_no,
                            protocol: Protocol::MessageQueue,
                            direction: FlowDirection::Stop,
                            key: q.to_string(),
                            method: String::new(),
                            framework: "rabbitmq".to_string(),
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
// KotlinRestConnector — HTTP client call starts + route stops for Kotlin
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
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();
        extract_kotlin_rest_stops(conn, &mut points)?;
        extract_kotlin_rest_starts(conn, project_root, &mut points)?;
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

fn extract_kotlin_rest_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // Retrofit: @GET("path"), @POST("path"), @PUT, @DELETE, @PATCH, @HEAD
    let re_retrofit = regex::Regex::new(
        r#"@(?P<method>GET|POST|PUT|DELETE|PATCH|HEAD)\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("kotlin retrofit annotation regex");

    // OkHttp: .url("path")
    let re_okhttp = regex::Regex::new(r#"\.url\s*\(\s*"(?P<url>[^"]+)"\s*\)"#)
        .expect("kotlin okhttp url regex");

    // Ktor client: client.get("path"), client.post("path"), etc.
    let re_ktor = regex::Regex::new(
        r#"(?:client|httpClient)\s*\.\s*(?P<method>get|post|put|delete|patch|head)\s*(?:<[^>]*>)?\s*\(\s*"(?P<url>[^"]+)""#,
    )
    .expect("kotlin ktor client regex");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language = 'kotlin'")
        .context("Failed to prepare Kotlin files query")?;
    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query Kotlin files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect Kotlin file rows")?;

    for (file_id, rel_path) in files {
        if kotlin_rest_is_test_file(&rel_path) {
            continue;
        }
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;

            // Retrofit annotations
            for cap in re_retrofit.captures_iter(line_text) {
                let raw_url = cap["url"].to_string();
                if !kotlin_rest_looks_like_api_url(&raw_url) {
                    continue;
                }
                let method = cap["method"].to_string();
                let url_pattern = rest_normalise_url_pattern(&raw_url);
                out.push(ConnectionPoint {
                    file_id,
                    symbol_id: None,
                    line: line_no,
                    protocol: Protocol::Rest,
                    direction: FlowDirection::Start,
                    key: url_pattern,
                    method,
                    framework: "retrofit".to_string(),
                    metadata: None,
                });
            }

            // OkHttp .url("path")
            if let Some(cap) = re_okhttp.captures(line_text) {
                let raw_url = cap["url"].to_string();
                if kotlin_rest_looks_like_api_url(&raw_url) {
                    let url_pattern = rest_normalise_url_pattern(&raw_url);
                    out.push(ConnectionPoint {
                        file_id,
                        symbol_id: None,
                        line: line_no,
                        protocol: Protocol::Rest,
                        direction: FlowDirection::Start,
                        key: url_pattern,
                        method: "GET".to_string(),
                        framework: "okhttp".to_string(),
                        metadata: None,
                    });
                }
            }

            // Ktor client
            for cap in re_ktor.captures_iter(line_text) {
                let raw_url = cap["url"].to_string();
                if !kotlin_rest_looks_like_api_url(&raw_url) {
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
                    framework: "ktor".to_string(),
                    metadata: None,
                });
            }
        }
    }
    Ok(())
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
