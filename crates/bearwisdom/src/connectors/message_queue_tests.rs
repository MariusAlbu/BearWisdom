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
