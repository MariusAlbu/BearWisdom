// =============================================================================
// connectors/message_queue.rs  —  Message queue publish/subscribe connector
//
// Detects producer and consumer sites for:
//   - Apache Kafka  (producer.send / kafkaTemplate.send / @KafkaListener /
//                    consumer.subscribe)
//   - RabbitMQ      (routing_key= / queue= / @RabbitListener)
//   - NATS          (nc.subscribe / nc.publish)
//   - AWS SQS       (sqs.send_message / SqsClient.send_message QueueUrl)
//
// Each site is represented as a `QueueEndpoint` (file_id, line, topic name,
// role, framework).  Producers and consumers that share the same topic/queue
// name are linked via `flow_edges` with edge_type = 'message_queue'.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

use crate::db::Database;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single message queue producer or consumer site.
#[derive(Debug, Clone)]
pub struct QueueEndpoint {
    /// `files.id` of the file containing the site.
    pub file_id: i64,
    /// 1-based line number.
    pub line: u32,
    /// The topic or queue name as a literal string.
    pub topic_or_queue: String,
    /// `"producer"` or `"consumer"`.
    pub role: String,
    /// `"kafka"`, `"rabbitmq"`, `"sqs"`, or `"nats"`.
    pub framework: String,
}

// ---------------------------------------------------------------------------
// Regex definitions
// ---------------------------------------------------------------------------

struct Patterns {
    // Kafka
    kafka_producer_send: Regex,
    kafka_template_send: Regex,
    kafka_listener: Regex,
    kafka_consumer_subscribe: Regex,
    // RabbitMQ
    rabbit_routing_key: Regex,
    rabbit_queue: Regex,
    rabbit_listener: Regex,
    // NATS
    nats_subscribe: Regex,
    nats_publish: Regex,
    // SQS
    sqs_send: Regex,
}

impl Patterns {
    fn build() -> Self {
        Self {
            // producer.send(topic="my-topic", ...) or producer.send("my-topic", ...)
            kafka_producer_send: Regex::new(
                r#"producer\.send\s*\(\s*(?:topic\s*=\s*)?['"]([^'"]+)['"]"#,
            )
            .expect("kafka_producer_send regex is valid"),

            // kafkaTemplate.send("my-topic", ...)
            kafka_template_send: Regex::new(
                r#"kafkaTemplate\.send\s*\(\s*['"]([^'"]+)['"]"#,
            )
            .expect("kafka_template_send regex is valid"),

            // @KafkaListener(topics = "my-topic") or @KafkaListener(topics = {"t1","t2"})
            kafka_listener: Regex::new(
                r#"@KafkaListener\s*\([^)]*topics\s*=\s*(?:\{[^}]*['"]([^'"]+)['"]|['"]([^'"]+)['"])"#,
            )
            .expect("kafka_listener regex is valid"),

            // consumer.subscribe(["my-topic"]) or consumer.subscribe("my-topic")
            kafka_consumer_subscribe: Regex::new(
                r#"consumer\.subscribe\s*\(\s*\[?\s*['"]([^'"]+)['"]"#,
            )
            .expect("kafka_consumer_subscribe regex is valid"),

            // routing_key="my.key" or routing_key='my.key'
            rabbit_routing_key: Regex::new(
                r#"routing_key\s*=\s*['"]([^'"]+)['"]"#,
            )
            .expect("rabbit_routing_key regex is valid"),

            // queue="my-queue" or queue='my-queue'
            rabbit_queue: Regex::new(
                r#"\bqueue\s*=\s*['"]([^'"]+)['"]"#,
            )
            .expect("rabbit_queue regex is valid"),

            // @RabbitListener(queues = "my-queue")
            rabbit_listener: Regex::new(
                r#"@RabbitListener\s*\([^)]*queues\s*=\s*(?:\{[^}]*['"]([^'"]+)['"]|['"]([^'"]+)['"])"#,
            )
            .expect("rabbit_listener regex is valid"),

            // nc.subscribe("subject")
            nats_subscribe: Regex::new(
                r#"nc\.subscribe\s*\(\s*['"]([^'"]+)['"]"#,
            )
            .expect("nats_subscribe regex is valid"),

            // nc.publish("subject", ...)
            nats_publish: Regex::new(
                r#"nc\.publish\s*\(\s*['"]([^'"]+)['"]"#,
            )
            .expect("nats_publish regex is valid"),

            // SQS: sqs.send_message(QueueUrl="https://...") or
            //      SqsClient.send_message(QueueUrl='...')
            sqs_send: Regex::new(
                r#"(?:sqs|SqsClient)\.send_message\s*\([^)]*QueueUrl\s*=\s*['"]([^'"]+)['"]"#,
            )
            .expect("sqs_send regex is valid"),
        }
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Scan all indexed source files for message queue producer/consumer patterns.
pub fn detect_queue_endpoints(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<QueueEndpoint>> {
    let patterns = Patterns::build();

    // Scan all supported languages.
    let mut stmt = conn
        .prepare(
            "SELECT id, path, language FROM files
             WHERE language IN (
                 'java', 'kotlin', 'python', 'typescript', 'tsx',
                 'javascript', 'jsx', 'go', 'csharp', 'rust'
             )",
        )
        .context("Failed to prepare source file query")?;

    let files: Vec<(i64, String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .context("Failed to query source files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect source file rows")?;

    let mut endpoints: Vec<QueueEndpoint> = Vec::new();

    for (file_id, rel_path, _language) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable file");
                continue;
            }
        };

        scan_file(&source, file_id, &patterns, &mut endpoints);
    }

    debug!(count = endpoints.len(), "Queue endpoints detected");
    Ok(endpoints)
}

fn scan_file(source: &str, file_id: i64, p: &Patterns, out: &mut Vec<QueueEndpoint>) {
    for (line_idx, line_text) in source.lines().enumerate() {
        let line_no = (line_idx + 1) as u32;

        // Kafka producers
        for cap in p.kafka_producer_send.captures_iter(line_text) {
            push_endpoint(out, file_id, line_no, &cap[1], "producer", "kafka");
        }
        for cap in p.kafka_template_send.captures_iter(line_text) {
            push_endpoint(out, file_id, line_no, &cap[1], "producer", "kafka");
        }

        // Kafka consumers
        for cap in p.kafka_listener.captures_iter(line_text) {
            let topic = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str());
            if let Some(t) = topic {
                push_endpoint(out, file_id, line_no, t, "consumer", "kafka");
            }
        }
        for cap in p.kafka_consumer_subscribe.captures_iter(line_text) {
            push_endpoint(out, file_id, line_no, &cap[1], "consumer", "kafka");
        }

        // RabbitMQ producers
        for cap in p.rabbit_routing_key.captures_iter(line_text) {
            push_endpoint(out, file_id, line_no, &cap[1], "producer", "rabbitmq");
        }
        for cap in p.rabbit_queue.captures_iter(line_text) {
            push_endpoint(out, file_id, line_no, &cap[1], "producer", "rabbitmq");
        }

        // RabbitMQ consumers
        for cap in p.rabbit_listener.captures_iter(line_text) {
            let queue = cap.get(1).or_else(|| cap.get(2)).map(|m| m.as_str());
            if let Some(q) = queue {
                push_endpoint(out, file_id, line_no, q, "consumer", "rabbitmq");
            }
        }

        // NATS
        for cap in p.nats_subscribe.captures_iter(line_text) {
            push_endpoint(out, file_id, line_no, &cap[1], "consumer", "nats");
        }
        for cap in p.nats_publish.captures_iter(line_text) {
            push_endpoint(out, file_id, line_no, &cap[1], "producer", "nats");
        }

        // SQS
        for cap in p.sqs_send.captures_iter(line_text) {
            push_endpoint(out, file_id, line_no, &cap[1], "producer", "sqs");
        }
    }
}

fn push_endpoint(
    out: &mut Vec<QueueEndpoint>,
    file_id: i64,
    line: u32,
    topic: &str,
    role: &str,
    framework: &str,
) {
    out.push(QueueEndpoint {
        file_id,
        line,
        topic_or_queue: topic.to_string(),
        role: role.to_string(),
        framework: framework.to_string(),
    });
}

// ---------------------------------------------------------------------------
// Linking
// ---------------------------------------------------------------------------

/// Match producers to consumers by exact topic/queue name and insert
/// `flow_edges` rows.  Returns the number of edges created.
pub fn link_producers_to_consumers(
    conn: &Connection,
    endpoints: &[QueueEndpoint],
) -> Result<u32> {
    if endpoints.is_empty() {
        return Ok(0);
    }

    let producers: Vec<&QueueEndpoint> = endpoints
        .iter()
        .filter(|e| e.role == "producer")
        .collect();

    let consumers: Vec<&QueueEndpoint> = endpoints
        .iter()
        .filter(|e| e.role == "consumer")
        .collect();

    if producers.is_empty() || consumers.is_empty() {
        return Ok(0);
    }

    let mut created: u32 = 0;

    for producer in &producers {
        for consumer in consumers.iter().filter(|c| {
            c.topic_or_queue == producer.topic_or_queue && c.framework == producer.framework
        }) {
            let result = conn.execute(
                "INSERT OR IGNORE INTO flow_edges (
                    source_file_id, source_line, source_symbol, source_language,
                    target_file_id, target_line, target_symbol, target_language,
                    edge_type, protocol, confidence
                 ) VALUES (
                    ?1, ?2, ?3, NULL,
                    ?4, ?5, ?6, NULL,
                    'message_queue', ?7, 0.85
                 )",
                rusqlite::params![
                    producer.file_id,
                    producer.line,
                    producer.topic_or_queue,
                    consumer.file_id,
                    consumer.line,
                    consumer.topic_or_queue,
                    producer.framework,
                ],
            );

            match result {
                Ok(n) if n > 0 => created += 1,
                Ok(_) => {}
                Err(e) => {
                    debug!(
                        err = %e,
                        topic = %producer.topic_or_queue,
                        "Failed to insert message_queue flow_edge"
                    );
                }
            }
        }
    }

    info!(created, "Message queue: producer→consumer edges created");
    Ok(created)
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run all message queue detection passes and write results to the database.
pub fn connect(db: &Database, project_root: &Path) -> Result<()> {
    let conn = &db.conn;

    let endpoints = detect_queue_endpoints(conn, project_root)
        .context("Message queue endpoint detection failed")?;
    info!(count = endpoints.len(), "Queue endpoints detected");

    let edges = link_producers_to_consumers(conn, &endpoints)
        .context("Message queue producer→consumer linking failed")?;
    info!(edges, "Message queue connector complete");

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_file(content: &str, suffix: &str) -> NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(suffix).tempfile().unwrap();
        write!(f, "{}", content).unwrap();
        f
    }

    fn insert_file(conn: &Connection, name: &str, lang: &str) -> i64 {
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES (?1, 'h', ?2, 0)",
            rusqlite::params![name, lang],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    // -----------------------------------------------------------------------
    // Regex unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn kafka_producer_send_regex_matches() {
        let p = Patterns::build();
        let line = r#"producer.send("order-events", message)"#;
        let cap = p.kafka_producer_send.captures(line).unwrap();
        assert_eq!(&cap[1], "order-events");
    }

    #[test]
    fn kafka_listener_annotation_matches() {
        let p = Patterns::build();
        let line = r#"@KafkaListener(topics = "order-events", groupId = "my-group")"#;
        let cap = p.kafka_listener.captures(line).unwrap();
        let topic = cap.get(1).or_else(|| cap.get(2)).unwrap().as_str();
        assert_eq!(topic, "order-events");
    }

    #[test]
    fn nats_subscribe_matches() {
        let p = Patterns::build();
        let line = r#"nc.subscribe("events.orders", handler)"#;
        let cap = p.nats_subscribe.captures(line).unwrap();
        assert_eq!(&cap[1], "events.orders");
    }

    #[test]
    fn nats_publish_matches() {
        let p = Patterns::build();
        let line = r#"nc.publish("events.orders", data)"#;
        let cap = p.nats_publish.captures(line).unwrap();
        assert_eq!(&cap[1], "events.orders");
    }

    #[test]
    fn rabbit_routing_key_matches() {
        let p = Patterns::build();
        let line = r#"channel.basic_publish(exchange='logs', routing_key='error.queue', body=msg)"#;
        let cap = p.rabbit_routing_key.captures(line).unwrap();
        assert_eq!(&cap[1], "error.queue");
    }

    // -----------------------------------------------------------------------
    // Integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn detect_kafka_producer_and_consumer() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        let producer = make_file(
            r#"producer.send("my-topic", event);"#,
            ".java",
        );
        let consumer = make_file(
            r#"@KafkaListener(topics = "my-topic", groupId = "g")"#,
            ".java",
        );

        let root = producer.path().parent().unwrap();
        insert_file(conn, producer.path().file_name().unwrap().to_str().unwrap(), "java");
        insert_file(conn, consumer.path().file_name().unwrap().to_str().unwrap(), "java");

        let endpoints = detect_queue_endpoints(conn, root).unwrap();

        let producers: Vec<_> = endpoints.iter().filter(|e| e.role == "producer").collect();
        let consumers: Vec<_> = endpoints.iter().filter(|e| e.role == "consumer").collect();

        assert_eq!(producers.len(), 1);
        assert_eq!(consumers.len(), 1);
        assert_eq!(producers[0].topic_or_queue, "my-topic");
        assert_eq!(consumers[0].topic_or_queue, "my-topic");
        assert_eq!(producers[0].framework, "kafka");
    }

    #[test]
    fn link_producers_creates_flow_edge() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        let producer_file_id = insert_file(conn, "producer.py", "python");
        let consumer_file_id = insert_file(conn, "consumer.py", "python");

        let endpoints = vec![
            QueueEndpoint {
                file_id: producer_file_id,
                line: 5,
                topic_or_queue: "orders".to_string(),
                role: "producer".to_string(),
                framework: "kafka".to_string(),
            },
            QueueEndpoint {
                file_id: consumer_file_id,
                line: 10,
                topic_or_queue: "orders".to_string(),
                role: "consumer".to_string(),
                framework: "kafka".to_string(),
            },
        ];

        let created = link_producers_to_consumers(conn, &endpoints).unwrap();
        assert_eq!(created, 1, "Expected one message_queue flow_edge");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'message_queue'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn no_match_different_topics_creates_no_edge() {
        let db = Database::open_in_memory().unwrap();
        let conn = &db.conn;

        let prod_id = insert_file(conn, "prod.java", "java");
        let cons_id = insert_file(conn, "cons.java", "java");

        let endpoints = vec![
            QueueEndpoint {
                file_id: prod_id,
                line: 1,
                topic_or_queue: "topic-a".to_string(),
                role: "producer".to_string(),
                framework: "kafka".to_string(),
            },
            QueueEndpoint {
                file_id: cons_id,
                line: 1,
                topic_or_queue: "topic-b".to_string(),
                role: "consumer".to_string(),
                framework: "kafka".to_string(),
            },
        ];

        let created = link_producers_to_consumers(conn, &endpoints).unwrap();
        assert_eq!(created, 0);
    }

    #[test]
    fn connect_runs_without_error_on_empty_project() {
        let db = Database::open_in_memory().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        connect(&db, dir.path()).unwrap();
    }
}
