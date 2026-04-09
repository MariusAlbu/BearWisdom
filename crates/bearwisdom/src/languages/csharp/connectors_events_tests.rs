use super::*;
use crate::db::Database;

// -----------------------------------------------------------------------
// Unit tests for helpers
// -----------------------------------------------------------------------

#[test]
fn handler_regex_matches_basic_form() {
    let re = build_handler_regex();
    let line = "public class OrderCreatedHandler : IIntegrationEventHandler<OrderCreatedIntegrationEvent>";
    let caps = re.captures(line).unwrap();
    assert_eq!(&caps[1], "OrderCreatedIntegrationEvent");
}

#[test]
fn handler_regex_matches_with_spaces() {
    let re = build_handler_regex();
    let line = "class Handler : IIntegrationEventHandler< OrderPlaced >";
    let caps = re.captures(line).unwrap();
    assert_eq!(&caps[1], "OrderPlaced");
}

#[test]
fn extract_class_name_finds_class() {
    assert_eq!(
        extract_class_name_from_line("public class OrderHandler : IFoo"),
        Some("OrderHandler".to_string())
    );
}

#[test]
fn extract_class_name_returns_none_for_no_class() {
    assert!(extract_class_name_from_line("// just a comment").is_none());
}

// -----------------------------------------------------------------------
// Integration tests
// -----------------------------------------------------------------------

fn seed_event_and_handler(db: &Database) -> (i64, i64) {
    let conn = db.conn();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('Events/OrderCreatedEvent.cs', 'h1', 'csharp', 0)",
        [],
    )
    .unwrap();
    let event_file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'IntegrationEvent', 'eShop.IntegrationEvent', 'class', 1, 0)",
        [event_file_id],
    )
    .unwrap();
    let base_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'OrderCreatedIntegrationEvent', 'eShop.OrderCreatedIntegrationEvent', 'class', 5, 0)",
        [event_file_id],
    )
    .unwrap();
    let event_sym_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, confidence)
         VALUES (?1, ?2, 'inherits', 1.0)",
        rusqlite::params![event_sym_id, base_id],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('Handlers/OrderCreatedHandler.cs', 'h2', 'csharp', 0)",
        [],
    )
    .unwrap();
    let handler_file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'OrderCreatedHandler', 'eShop.OrderCreatedHandler', 'class', 3, 0)",
        [handler_file_id],
    )
    .unwrap();
    let handler_sym_id: i64 = conn.last_insert_rowid();

    (event_sym_id, handler_sym_id)
}

#[test]
fn find_integration_events_detects_via_edges() {
    let db = Database::open_in_memory().unwrap();
    seed_event_and_handler(&db);

    let events = find_integration_events(db.conn()).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "OrderCreatedIntegrationEvent");
}

#[test]
fn link_events_to_handlers_inserts_flow_edge() {
    let db = Database::open_in_memory().unwrap();
    seed_event_and_handler(&db);

    let events = find_integration_events(db.conn()).unwrap();

    let handlers = vec![EventHandler {
        symbol_id: 99,
        name: "OrderCreatedHandler".to_string(),
        event_type: "OrderCreatedIntegrationEvent".to_string(),
        file_path: "Handlers/OrderCreatedHandler.cs".to_string(),
    }];

    let created = link_events_to_handlers(db.conn(), &events, &handlers).unwrap();
    assert_eq!(created, 1, "Expected one flow_edge");

    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'event_handler'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn no_matching_event_creates_no_edge() {
    let db = Database::open_in_memory().unwrap();
    seed_event_and_handler(&db);

    let events = find_integration_events(db.conn()).unwrap();

    let handlers = vec![EventHandler {
        symbol_id: 1,
        name: "SomeHandler".to_string(),
        event_type: "NonExistentEvent".to_string(),
        file_path: "Handlers/OrderCreatedHandler.cs".to_string(),
    }];

    let created = link_events_to_handlers(db.conn(), &events, &handlers).unwrap();
    assert_eq!(created, 0);
}

#[test]
fn empty_inputs_return_zero() {
    let db = Database::open_in_memory().unwrap();
    let created = link_events_to_handlers(db.conn(), &[], &[]).unwrap();
    assert_eq!(created, 0);
}
