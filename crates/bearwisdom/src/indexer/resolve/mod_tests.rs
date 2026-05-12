// Tests for flush_flow_emissions — pairing logic in indexer/resolve/mod.rs.
//
// Each test opens an in-memory Database (full schema), inserts a minimal
// `files` row for every path used in the emissions, calls the internal
// `_test_flush_flow_emissions` wrapper, then queries `flow_edges` to assert
// the expected pairing outcome.

use crate::db::Database;
use crate::indexer::resolve::flow_emit::{
    AuthGuardKind, ChannelRole, DbQueryOp, FlowEmission, HttpMethod, MigrationDirection,
    NamedChannelKind,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Insert a `files` row and return its id.
fn insert_file(db: &Database, path: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, origin) \
             VALUES (?1, 'testhash', 'typescript', 0, 'internal')",
            rusqlite::params![path],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

/// Count paired rows (target_file_id IS NOT NULL).
fn count_paired(db: &Database) -> i64 {
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE target_file_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// Count single-ended rows (target_file_id IS NULL).
fn count_single(db: &Database) -> i64 {
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE target_file_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// Count total flow_edges rows.
fn count_total(db: &Database) -> i64 {
    db.conn()
        .query_row("SELECT COUNT(*) FROM flow_edges", [], |r| r.get(0))
        .unwrap()
}

// ---------------------------------------------------------------------------
// NamedChannel pairing
// ---------------------------------------------------------------------------

#[test]
fn named_channel_producer_consumer_same_name_pairs() {
    let mut db = Database::open_in_memory().unwrap();
    let _f1 = insert_file(&db, "/app/frontend/api.ts");
    let _f2 = insert_file(&db, "/app/backend/users.ts");

    let emissions = vec![
        (
            "/app/frontend/api.ts".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/users".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Get),
            },
        ),
        (
            "/app/backend/users.ts".to_string(),
            42u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/users".to_string(),
                role: ChannelRole::Consumer,
                method: Some(HttpMethod::Get),
            },
        ),
    ];

    let written = super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(written, 2, "both sides of the pair counted");
    assert_eq!(count_paired(&db), 1);
    assert_eq!(count_single(&db), 0);
}

#[test]
fn named_channel_no_pair_when_both_same_role() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/app/a.ts");
    insert_file(&db, "/app/b.ts");

    let emissions = vec![
        (
            "/app/a.ts".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/data".to_string(),
                role: ChannelRole::Producer,
                method: None,
            },
        ),
        (
            "/app/b.ts".to_string(),
            2u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/data".to_string(),
                role: ChannelRole::Producer, // both producers — no pair
                method: None,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 0);
    assert_eq!(count_single(&db), 2);
}

// ---------------------------------------------------------------------------
// URL-pattern normalization through pairing
// ---------------------------------------------------------------------------

#[test]
fn named_channel_pairs_across_url_param_syntax() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/frontend/client.ts");
    insert_file(&db, "/backend/controller.ts");

    // Producer uses canonical `{}`, consumer uses NestJS `:id`.
    let emissions = vec![
        (
            "/frontend/client.ts".to_string(),
            5u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/users/{}".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Get),
            },
        ),
        (
            "/backend/controller.ts".to_string(),
            20u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/users/:id".to_string(),
                role: ChannelRole::Consumer,
                method: Some(HttpMethod::Get),
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1, "normalized URLs must pair");
}

#[test]
fn named_channel_pairs_fastapi_brace_against_express_colon() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/fe/api.ts");
    insert_file(&db, "/be/router.ts");

    let emissions = vec![
        (
            "/fe/api.ts".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/items/{itemId}".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Put),
            },
        ),
        (
            "/be/router.ts".to_string(),
            2u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/items/:itemId".to_string(),
                role: ChannelRole::Consumer,
                method: Some(HttpMethod::Put),
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1);
}

// ---------------------------------------------------------------------------
// HTTP method compatibility
// ---------------------------------------------------------------------------

#[test]
fn http_method_any_producer_pairs_with_concrete_consumer() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/fe/api.ts");
    insert_file(&db, "/be/handler.ts");

    let emissions = vec![
        (
            "/fe/api.ts".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/items".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Any), // wildcard
            },
        ),
        (
            "/be/handler.ts".to_string(),
            2u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/items".to_string(),
                role: ChannelRole::Consumer,
                method: Some(HttpMethod::Get),
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1, "Any should match GET");
}

#[test]
fn http_method_mismatch_no_pair() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/fe/api.ts");
    insert_file(&db, "/be/handler.ts");

    let emissions = vec![
        (
            "/fe/api.ts".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/items".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Post),
            },
        ),
        (
            "/be/handler.ts".to_string(),
            2u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/items".to_string(),
                role: ChannelRole::Consumer,
                method: Some(HttpMethod::Get),
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    // POST ≠ GET — no pair, both single-ended.
    assert_eq!(count_paired(&db), 0);
    assert_eq!(count_single(&db), 2);
}

// ---------------------------------------------------------------------------
// DbEntity ↔ DbQuery pairing
// ---------------------------------------------------------------------------

#[test]
fn db_entity_query_pair_exact_name() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/src/user.entity.ts");
    insert_file(&db, "/src/user.repository.ts");

    let emissions = vec![
        (
            "/src/user.entity.ts".to_string(),
            10u32,
            FlowEmission::DbEntity {
                base_symbol_id: None,
                base_name_hint: "Entity".to_string(),
                table_name_hint: Some("users".to_string()),
            },
        ),
        (
            "/src/user.repository.ts".to_string(),
            25u32,
            FlowEmission::DbQuery {
                entity_name: "users".to_string(),
                operation: DbQueryOp::Select,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1, "exact table name should pair");
}

#[test]
fn db_entity_query_pair_case_insensitive() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/src/user.ts");
    insert_file(&db, "/src/repo.ts");

    let emissions = vec![
        (
            "/src/user.ts".to_string(),
            1u32,
            FlowEmission::DbEntity {
                base_symbol_id: None,
                base_name_hint: "Model".to_string(),
                table_name_hint: Some("User".to_string()),
            },
        ),
        (
            "/src/repo.ts".to_string(),
            2u32,
            FlowEmission::DbQuery {
                entity_name: "user".to_string(), // lowercase
                operation: DbQueryOp::Update,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1);
}

#[test]
fn db_entity_query_pair_pluralization_class_to_table() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/src/order.ts");
    insert_file(&db, "/src/order.service.ts");

    // Entity class name "Order", query uses "orders" (pluralised table name).
    let emissions = vec![
        (
            "/src/order.ts".to_string(),
            1u32,
            FlowEmission::DbEntity {
                base_symbol_id: None,
                base_name_hint: "Order".to_string(),
                table_name_hint: None,
            },
        ),
        (
            "/src/order.service.ts".to_string(),
            5u32,
            FlowEmission::DbQuery {
                entity_name: "orders".to_string(),
                operation: DbQueryOp::Insert,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1);
}

#[test]
fn db_entity_multiple_queries_each_get_a_row() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/src/post.ts");
    insert_file(&db, "/src/post.controller.ts");
    insert_file(&db, "/src/post.service.ts");

    let emissions = vec![
        (
            "/src/post.ts".to_string(),
            1u32,
            FlowEmission::DbEntity {
                base_symbol_id: None,
                base_name_hint: "Post".to_string(),
                table_name_hint: Some("posts".to_string()),
            },
        ),
        (
            "/src/post.controller.ts".to_string(),
            10u32,
            FlowEmission::DbQuery {
                entity_name: "posts".to_string(),
                operation: DbQueryOp::Select,
            },
        ),
        (
            "/src/post.service.ts".to_string(),
            20u32,
            FlowEmission::DbQuery {
                entity_name: "posts".to_string(),
                operation: DbQueryOp::Insert,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    // Entity + 2 queries → 2 paired edges, entity row appears as target in both.
    assert_eq!(count_paired(&db), 2);
}

// ---------------------------------------------------------------------------
// MigrationTarget ↔ DbEntity pairing
// ---------------------------------------------------------------------------

#[test]
fn migration_target_pairs_with_db_entity() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/src/user.entity.ts");
    insert_file(&db, "/migrations/20240101_create_users.ts");

    let emissions = vec![
        (
            "/src/user.entity.ts".to_string(),
            1u32,
            FlowEmission::DbEntity {
                base_symbol_id: None,
                base_name_hint: "User".to_string(),
                table_name_hint: Some("users".to_string()),
            },
        ),
        (
            "/migrations/20240101_create_users.ts".to_string(),
            5u32,
            FlowEmission::MigrationTarget {
                table_name: "users".to_string(),
                direction: MigrationDirection::Up,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1);
}

#[test]
fn three_way_db_entity_query_migration() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/src/product.ts");
    insert_file(&db, "/src/product.repo.ts");
    insert_file(&db, "/migrations/create_products.ts");

    let emissions = vec![
        (
            "/src/product.ts".to_string(),
            1u32,
            FlowEmission::DbEntity {
                base_symbol_id: None,
                base_name_hint: "Product".to_string(),
                table_name_hint: Some("products".to_string()),
            },
        ),
        (
            "/src/product.repo.ts".to_string(),
            10u32,
            FlowEmission::DbQuery {
                entity_name: "products".to_string(),
                operation: DbQueryOp::Select,
            },
        ),
        (
            "/migrations/create_products.ts".to_string(),
            5u32,
            FlowEmission::MigrationTarget {
                table_name: "products".to_string(),
                direction: MigrationDirection::Up,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    // DbQuery→DbEntity edge + MigrationTarget→DbEntity edge = 2 paired rows.
    assert_eq!(count_paired(&db), 2);
    assert_eq!(count_single(&db), 0);
}

// ---------------------------------------------------------------------------
// Single-ended variants (target_file_id IS NULL)
// ---------------------------------------------------------------------------

#[test]
fn single_ended_variants_write_without_target_file_id() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/src/app.ts");

    let emissions = vec![
        (
            "/src/app.ts".to_string(),
            1u32,
            FlowEmission::DiBinding {
                service_symbol_id: 99,
                container: Some("nestjs".to_string()),
            },
        ),
        (
            "/src/app.ts".to_string(),
            2u32,
            FlowEmission::ConfigLookup { key: "DATABASE_URL".to_string() },
        ),
        (
            "/src/app.ts".to_string(),
            3u32,
            FlowEmission::FeatureFlag { flag_name: "new_dashboard".to_string() },
        ),
        (
            "/src/app.ts".to_string(),
            4u32,
            FlowEmission::AuthGuard {
                requirement: "admin".to_string(),
                kind: AuthGuardKind::Role,
            },
        ),
        (
            "/src/app.ts".to_string(),
            5u32,
            FlowEmission::CliCommand {
                command_name: "build".to_string(),
                framework: None,
            },
        ),
        (
            "/src/app.ts".to_string(),
            6u32,
            FlowEmission::ScheduledJob {
                schedule: "0 * * * *".to_string(),
            },
        ),
    ];

    let written = super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(written, 6);
    assert_eq!(count_single(&db), 6);
    assert_eq!(count_paired(&db), 0);
}

// ---------------------------------------------------------------------------
// Duplicate detection
// ---------------------------------------------------------------------------

#[test]
fn duplicate_emissions_produce_single_edge() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/fe/api.ts");
    insert_file(&db, "/be/users.ts");

    // Producer emitted twice (e.g. same call site resolved in two passes).
    let emission_pair = FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: "/api/users".to_string(),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Get),
    };
    let consumer = FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: "/api/users".to_string(),
        role: ChannelRole::Consumer,
        method: Some(HttpMethod::Get),
    };

    let emissions = vec![
        ("/fe/api.ts".to_string(), 10u32, emission_pair.clone()),
        ("/fe/api.ts".to_string(), 10u32, emission_pair), // exact duplicate
        ("/be/users.ts".to_string(), 42u32, consumer),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    // INSERT OR IGNORE deduplicates the identical edge.
    assert_eq!(count_paired(&db), 1);
}

// ---------------------------------------------------------------------------
// Empty-name NamedChannel falls through to single-ended
// ---------------------------------------------------------------------------

#[test]
fn named_channel_empty_name_written_as_single_ended() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/src/api.ts");

    let emissions = vec![(
        "/src/api.ts".to_string(),
        5u32,
        FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: String::new(), // unknown URL — cannot pair
            role: ChannelRole::Producer,
            method: None,
        },
    )];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_single(&db), 1);
    assert_eq!(count_paired(&db), 0);
}

// ---------------------------------------------------------------------------
// Files not in DB are skipped gracefully
// ---------------------------------------------------------------------------

#[test]
fn emission_for_unknown_file_is_silently_skipped() {
    let mut db = Database::open_in_memory().unwrap();
    // Deliberately NOT inserting any files row.

    let emissions = vec![(
        "/nonexistent/file.ts".to_string(),
        1u32,
        FlowEmission::ConfigLookup { key: "KEY".to_string() },
    )];

    let written = super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(written, 0);
    assert_eq!(count_total(&db), 0);
}
