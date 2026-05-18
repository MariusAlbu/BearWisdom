// Tests for flush_flow_emissions — pairing logic in indexer/resolve/mod.rs.
//
// Each test opens an in-memory Database (full schema), inserts a minimal
// `files` row for every path used in the emissions, calls the internal
// `_test_flush_flow_emissions` wrapper, then queries `flow_edges` to assert
// the expected pairing outcome.

use crate::db::Database;
use crate::indexer::resolve::flow_emit::{
    AuthGuardKind, ChannelRole, DbQueryOp, FlowEmission, HttpMethod, MigrationDirection,
    NamedChannelKind, StreamKind,
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
            streaming: None,
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
            streaming: None,
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
            streaming: None,
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
            streaming: None,
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
            streaming: None,
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
            streaming: None,
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
            streaming: None,
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
            streaming: None,
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
                streaming: None,
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
            streaming: None,
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
            streaming: None,
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
            streaming: None,
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
    streaming: None,
    };
    let consumer = FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: "/api/users".to_string(),
        role: ChannelRole::Consumer,
        method: Some(HttpMethod::Get),
    streaming: None,
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
        streaming: None,
        },
    )];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_single(&db), 1);
    assert_eq!(count_paired(&db), 0);
}

// ---------------------------------------------------------------------------
// Wildcard NamedChannel pairing
//
// `<prefix>/*` on one role matches every concrete `<prefix>/<x>` emission on
// the opposite role. Modeled after background-job Consumer constructors
// (`new Worker('queue', …)` → `queue/*`) that should pair with every
// concrete `queue.add('job', …)` Producer for the same queue.
// ---------------------------------------------------------------------------

#[test]
fn wildcard_consumer_pairs_with_concrete_producer() {
    let mut db = Database::open_in_memory().unwrap();
    let _f1 = insert_file(&db, "/app/producer.ts");
    let _f2 = insert_file(&db, "/app/consumer.ts");

    // `queue.add('send-welcome', …)` produces `email/send-welcome`.
    // `new Worker('email', …)` consumes `email/*`. They must pair.
    let emissions = vec![
        (
            "/app/producer.ts".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::BgJob,
                name: "email/send-welcome".to_string(),
                role: ChannelRole::Producer,
                method: None,
            streaming: None,
            },
        ),
        (
            "/app/consumer.ts".to_string(),
            20u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::BgJob,
                name: "email/*".to_string(),
                role: ChannelRole::Consumer,
                method: None,
            streaming: None,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1, "wildcard consumer should pair with concrete producer");
    assert_eq!(count_single(&db), 0, "no single-ended rows when wildcard pairs");

    // The paired edge keeps the concrete url_pattern (more searchable than
    // the wildcard form); normalize prepends a leading `/`.
    let url: String = db
        .conn()
        .query_row(
            "SELECT url_pattern FROM flow_edges WHERE target_file_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(url, "/email/send-welcome");
}

#[test]
fn wildcard_consumer_pairs_with_multiple_concrete_producers() {
    let mut db = Database::open_in_memory().unwrap();
    let _f1 = insert_file(&db, "/app/p1.ts");
    let _f2 = insert_file(&db, "/app/p2.ts");
    let _f3 = insert_file(&db, "/app/worker.ts");

    // One Worker, two concrete `queue.add` call sites. Both pair.
    let emissions = vec![
        (
            "/app/p1.ts".to_string(),
            5u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::BgJob,
                name: "email/send-welcome".to_string(),
                role: ChannelRole::Producer,
                method: None,
            streaming: None,
            },
        ),
        (
            "/app/p2.ts".to_string(),
            8u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::BgJob,
                name: "email/send-reset".to_string(),
                role: ChannelRole::Producer,
                method: None,
            streaming: None,
            },
        ),
        (
            "/app/worker.ts".to_string(),
            12u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::BgJob,
                name: "email/*".to_string(),
                role: ChannelRole::Consumer,
                method: None,
            streaming: None,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 2, "wildcard consumer pairs with each concrete producer");
    assert_eq!(count_single(&db), 0);
}

#[test]
fn wildcard_does_not_match_different_prefix() {
    let mut db = Database::open_in_memory().unwrap();
    let _f1 = insert_file(&db, "/app/producer.ts");
    let _f2 = insert_file(&db, "/app/worker.ts");

    // Different queue names — must not pair.
    let emissions = vec![
        (
            "/app/producer.ts".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::BgJob,
                name: "payment/charge".to_string(),
                role: ChannelRole::Producer,
                method: None,
            streaming: None,
            },
        ),
        (
            "/app/worker.ts".to_string(),
            20u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::BgJob,
                name: "email/*".to_string(),
                role: ChannelRole::Consumer,
                method: None,
            streaming: None,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 0);
    assert_eq!(count_single(&db), 2, "both fall through to single-ended rows");
}

#[test]
fn wildcard_consumer_pairs_within_same_edge_type_only() {
    let mut db = Database::open_in_memory().unwrap();
    let _f1 = insert_file(&db, "/app/http_producer.ts");
    let _f2 = insert_file(&db, "/app/bg_consumer.ts");

    // Same string prefix but different edge_type — must not pair.
    let emissions = vec![
        (
            "/app/http_producer.ts".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "email/send".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Post),
            streaming: None,
            },
        ),
        (
            "/app/bg_consumer.ts".to_string(),
            20u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::BgJob,
                name: "email/*".to_string(),
                role: ChannelRole::Consumer,
                method: None,
            streaming: None,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 0, "edge_type mismatch blocks wildcard pairing");
    assert_eq!(count_single(&db), 2);
}

// ---------------------------------------------------------------------------
// Mailer template-root file path detection
// ---------------------------------------------------------------------------

#[test]
fn test_mailer_template_recognised_under_mails_root() {
    let name = super::mailer_template_name_for_path("apps/api/src/mails/welcome.hbs");
    assert_eq!(name.as_deref(), Some("welcome"));
}

#[test]
fn test_mailer_template_recognised_under_emails_root() {
    let name = super::mailer_template_name_for_path("emails/Welcome.tsx");
    assert_eq!(name.as_deref(), Some("Welcome"));
}

#[test]
fn test_mailer_template_recognised_under_templates_email() {
    let name = super::mailer_template_name_for_path("server/templates/email/verify-email.hbs");
    assert_eq!(name.as_deref(), Some("verify-email"));
}

#[test]
fn test_mailer_template_recognised_under_views_mailers() {
    let name = super::mailer_template_name_for_path("app/views/mailers/user_mailer.html.erb");
    assert_eq!(name.as_deref(), Some("user_mailer.html"));
}

#[test]
fn test_mailer_template_not_recognised_outside_known_roots() {
    assert!(super::mailer_template_name_for_path("src/utils/email-helper.ts").is_none());
    assert!(super::mailer_template_name_for_path("docs/mail.md").is_none());
}

// ---------------------------------------------------------------------------
// ExtractedRoute → FlowEmission adapter
// ---------------------------------------------------------------------------

fn fake_route(handler_idx: usize, method: &str, template: &str) -> crate::types::ExtractedRoute {
    crate::types::ExtractedRoute {
        handler_symbol_index: handler_idx,
        http_method: method.to_string(),
        template: template.to_string(),
    }
}

#[test]
fn extracted_route_adapter_emits_consumer_with_method() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};

    let symbols = vec![fake_symbol("getUser")];
    let routes = vec![fake_route(0, "GET", "/api/users/foo")];
    let emissions = super::extracted_routes_to_emissions(&routes, &symbols);
    assert_eq!(emissions.len(), 1);
    match &emissions[0].1 {
        FlowEmission::NamedChannel { kind, role, name, method, .. } => {
            assert_eq!(*kind, NamedChannelKind::HttpCall);
            assert_eq!(*role, ChannelRole::Consumer);
            assert_eq!(name, "/api/users/foo");
            assert_eq!(*method, Some(HttpMethod::Get));
        }
        other => panic!("expected NamedChannel HttpCall, got {other:?}"),
    }
}

#[test]
fn extracted_route_adapter_normalizes_dynamic_segments() {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    // Three URL conventions, all normalize to `{}`.
    let symbols = vec![fake_symbol("h"); 3];
    let routes = vec![
        fake_route(0, "GET", "/api/users/{id}"),
        fake_route(1, "GET", "/api/users/:id"),
        fake_route(2, "GET", "/api/users/<id>"),
    ];
    let emissions = super::extracted_routes_to_emissions(&routes, &symbols);
    assert_eq!(emissions.len(), 3);
    for (_, emission) in &emissions {
        match emission {
            FlowEmission::NamedChannel { name, .. } => {
                assert_eq!(name, "/api/users/{}");
            }
            _ => panic!("expected NamedChannel"),
        }
    }
}

#[test]
fn extracted_route_adapter_skips_empty_template() {
    let symbols = vec![fake_symbol("h"); 3];
    let routes = vec![
        fake_route(0, "GET", ""),
        fake_route(1, "GET", "   "),
        fake_route(2, "POST", "/real"),
    ];
    let emissions = super::extracted_routes_to_emissions(&routes, &symbols);
    assert_eq!(emissions.len(), 1, "only the non-empty route emits");
}

// ---------------------------------------------------------------------------
// Segment-wildcard pairer pass
// ---------------------------------------------------------------------------

#[test]
fn segment_wildcard_consumer_pairs_with_concrete_producer() {
    let mut db = Database::open_in_memory().unwrap();
    let _f1 = insert_file(&db, "/app/client.tsx");
    let _f2 = insert_file(&db, "/app/api/trpc/[trpc]/route.ts");

    let emissions = vec![
        (
            "/app/client.tsx".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/trpc/polls.list".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Any),
            streaming: None,
            },
        ),
        (
            "/app/api/trpc/[trpc]/route.ts".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/trpc/{}".to_string(),
                role: ChannelRole::Consumer,
                method: Some(HttpMethod::Any),
            streaming: None,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1, "segment-wildcard Consumer pairs with concrete Producer");
    assert_eq!(count_single(&db), 0);
}

#[test]
fn segment_wildcard_pairs_multi_segment_paths() {
    let mut db = Database::open_in_memory().unwrap();
    let _f1 = insert_file(&db, "/app/client.tsx");
    let _f2 = insert_file(&db, "/app/api/users/[id]/posts/route.ts");

    let emissions = vec![
        (
            "/app/client.tsx".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/users/42/posts".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Get),
            streaming: None,
            },
        ),
        (
            "/app/api/users/[id]/posts/route.ts".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/users/{}/posts".to_string(),
                role: ChannelRole::Consumer,
                method: Some(HttpMethod::Get),
            streaming: None,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1);
}

#[test]
fn segment_wildcard_rejects_different_segment_count() {
    let mut db = Database::open_in_memory().unwrap();
    let _f1 = insert_file(&db, "/a.ts");
    let _f2 = insert_file(&db, "/b.ts");

    // Consumer `/api/users/{}` has 3 segments, Producer `/api/users` has 2.
    // Must not pair.
    let emissions = vec![
        (
            "/a.ts".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/users".to_string(),
                role: ChannelRole::Producer,
                method: Some(HttpMethod::Get),
            streaming: None,
            },
        ),
        (
            "/b.ts".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::HttpCall,
                name: "/api/users/{}".to_string(),
                role: ChannelRole::Consumer,
                method: Some(HttpMethod::Get),
            streaming: None,
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 0);
}

// ---------------------------------------------------------------------------
// Next.js route file-path Consumer detection
// ---------------------------------------------------------------------------

fn fake_symbol(name: &str) -> crate::types::ExtractedSymbol {
    crate::types::ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind: crate::types::SymbolKind::Function,
        visibility: None,
        start_line: 1,
        end_line: 1,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
    }
}

#[test]
fn test_nextjs_app_router_static_route_emits_per_verb_consumers() {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};

    let symbols = vec![fake_symbol("GET"), fake_symbol("POST")];
    let emissions = super::nextjs_route_consumer_emissions(
        "src/app/users/route.ts",
        &symbols,
    );
    assert_eq!(emissions.len(), 2);
    let urls: Vec<(String, Option<HttpMethod>)> = emissions
        .iter()
        .map(|e| match e {
            FlowEmission::NamedChannel { kind, role, name, method, .. } => {
                assert_eq!(*kind, NamedChannelKind::HttpCall);
                assert_eq!(*role, ChannelRole::Consumer);
                (name.clone(), *method)
            }
            _ => panic!("expected NamedChannel HttpCall"),
        })
        .collect();
    assert_eq!(urls[0], ("/users".to_string(), Some(HttpMethod::Get)));
    assert_eq!(urls[1], ("/users".to_string(), Some(HttpMethod::Post)));
}

#[test]
fn test_nextjs_app_router_dynamic_segment_rewritten() {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    let symbols = vec![fake_symbol("GET")];
    let emissions = super::nextjs_route_consumer_emissions(
        "src/app/users/[id]/route.ts",
        &symbols,
    );
    assert_eq!(emissions.len(), 1);
    match &emissions[0] {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/users/{}"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nextjs_app_router_catch_all_segment_rewritten() {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    let symbols = vec![fake_symbol("GET")];
    let emissions = super::nextjs_route_consumer_emissions(
        "src/app/docs/[...slug]/route.ts",
        &symbols,
    );
    assert_eq!(emissions.len(), 1);
    match &emissions[0] {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/docs/{}"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nextjs_app_router_optional_catch_all_segment_rewritten() {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    let symbols = vec![fake_symbol("GET")];
    let emissions = super::nextjs_route_consumer_emissions(
        "src/app/blog/[[...slug]]/route.ts",
        &symbols,
    );
    assert_eq!(emissions.len(), 1);
    match &emissions[0] {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/blog/{}"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nextjs_app_router_route_groups_collapsed() {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    // `(marketing)` is a route group and contributes no URL segment.
    let symbols = vec![fake_symbol("GET")];
    let emissions = super::nextjs_route_consumer_emissions(
        "app/(marketing)/about/route.ts",
        &symbols,
    );
    match &emissions[0] {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "//about"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nextjs_app_router_no_handlers_falls_back_to_any() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};

    // The Next.js App Router contract requires `route.ts` to export at least
    // one HTTP verb. When the verbs come from `export { x as GET, x as POST }`
    // and no synthetic symbol is emitted for the alias, fall back to a
    // single Any-method Consumer so the route still participates in
    // pairing.
    let symbols = vec![fake_symbol("helper"), fake_symbol("internalThing")];
    let emissions = super::nextjs_route_consumer_emissions(
        "app/users/route.ts",
        &symbols,
    );
    assert_eq!(emissions.len(), 1);
    match &emissions[0] {
        FlowEmission::NamedChannel { name, method, .. } => {
            assert_eq!(name, "/users");
            assert_eq!(*method, Some(HttpMethod::Any));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nextjs_pages_router_static_emits_any_method_consumer() {
    use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};

    let symbols: Vec<crate::types::ExtractedSymbol> = vec![];
    let emissions = super::nextjs_route_consumer_emissions(
        "pages/api/users.ts",
        &symbols,
    );
    assert_eq!(emissions.len(), 1);
    match &emissions[0] {
        FlowEmission::NamedChannel { name, method, .. } => {
            assert_eq!(name, "/api/users");
            assert_eq!(*method, Some(HttpMethod::Any));
        }
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nextjs_pages_router_dynamic_segment_rewritten() {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    let symbols: Vec<crate::types::ExtractedSymbol> = vec![];
    let emissions = super::nextjs_route_consumer_emissions(
        "pages/api/users/[id].ts",
        &symbols,
    );
    assert_eq!(emissions.len(), 1);
    match &emissions[0] {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/api/users/{}"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nextjs_pages_router_index_basename_collapsed() {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    let symbols: Vec<crate::types::ExtractedSymbol> = vec![];
    let emissions = super::nextjs_route_consumer_emissions(
        "pages/api/users/index.ts",
        &symbols,
    );
    match &emissions[0] {
        FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/api/users"),
        _ => panic!("expected NamedChannel"),
    }
}

#[test]
fn test_nextjs_pages_router_underscore_files_skipped() {
    let symbols: Vec<crate::types::ExtractedSymbol> = vec![];
    let emissions = super::nextjs_route_consumer_emissions(
        "pages/api/_middleware.ts",
        &symbols,
    );
    assert!(emissions.is_empty());
}

#[test]
fn test_nextjs_non_route_paths_emit_nothing() {
    let symbols: Vec<crate::types::ExtractedSymbol> = vec![];
    assert!(super::nextjs_route_consumer_emissions("src/lib/helper.ts", &symbols).is_empty());
    assert!(super::nextjs_route_consumer_emissions("src/app/page.tsx", &symbols).is_empty());
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

// ---------------------------------------------------------------------------
// gRPC streaming compatibility (Goal 71)
// ---------------------------------------------------------------------------

#[test]
fn unary_producer_pairs_with_unary_consumer() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/fe/client.rs");
    insert_file(&db, "/be/server.rs");

    let emissions = vec![
        (
            "/fe/client.rs".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::RpcCall,
                name: "User.GetUser".to_string(),
                role: ChannelRole::Producer,
                method: None,
                streaming: Some(StreamKind::Unary),
            },
        ),
        (
            "/be/server.rs".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::RpcCall,
                name: "User.GetUser".to_string(),
                role: ChannelRole::Consumer,
                method: None,
                streaming: Some(StreamKind::Unary),
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    let paired = count_paired(&db);
    assert_eq!(paired, 1, "unary producer must pair with unary consumer");
}

#[test]
fn unary_producer_does_not_pair_with_streaming_consumer() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/fe/client.rs");
    insert_file(&db, "/be/server.rs");

    let emissions = vec![
        (
            "/fe/client.rs".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::RpcCall,
                name: "Events.Stream".to_string(),
                role: ChannelRole::Producer,
                method: None,
                streaming: Some(StreamKind::Unary),
            },
        ),
        (
            "/be/server.rs".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::RpcCall,
                name: "Events.Stream".to_string(),
                role: ChannelRole::Consumer,
                method: None,
                streaming: Some(StreamKind::ServerStreaming),
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(
        count_paired(&db),
        0,
        "unary-vs-streaming pair must be rejected by streaming_kinds_compatible"
    );
}

#[test]
fn server_streaming_pairs_with_server_streaming() {
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/fe/client.rs");
    insert_file(&db, "/be/server.rs");

    let emissions = vec![
        (
            "/fe/client.rs".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::RpcCall,
                name: "Events.Stream".to_string(),
                role: ChannelRole::Producer,
                method: None,
                streaming: Some(StreamKind::ServerStreaming),
            },
        ),
        (
            "/be/server.rs".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::RpcCall,
                name: "Events.Stream".to_string(),
                role: ChannelRole::Consumer,
                method: None,
                streaming: Some(StreamKind::ServerStreaming),
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1);

    // Confirm metadata column carries the stream kind.
    let metadata: Option<String> = db
        .conn()
        .query_row(
            "SELECT metadata FROM flow_edges WHERE target_file_id IS NOT NULL LIMIT 1",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .unwrap();
    assert_eq!(metadata.as_deref(), Some("server_streaming"));
}

#[test]
fn none_streaming_pairs_with_unary_for_back_compat() {
    // Detectors that haven't been wired for streaming yet emit
    // `streaming: None`. Pairing must still succeed against `Some(Unary)`
    // — otherwise upgrading one side of a corpus would break all existing
    // RPC pairs.
    let mut db = Database::open_in_memory().unwrap();
    insert_file(&db, "/fe/client.rs");
    insert_file(&db, "/be/server.rs");

    let emissions = vec![
        (
            "/fe/client.rs".to_string(),
            1u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::RpcCall,
                name: "Greeter.Hello".to_string(),
                role: ChannelRole::Producer,
                method: None,
                streaming: None,
            },
        ),
        (
            "/be/server.rs".to_string(),
            10u32,
            FlowEmission::NamedChannel {
                kind: NamedChannelKind::RpcCall,
                name: "Greeter.Hello".to_string(),
                role: ChannelRole::Consumer,
                method: None,
                streaming: Some(StreamKind::Unary),
            },
        ),
    ];

    super::_test_flush_flow_emissions(db.conn(), &emissions).unwrap();
    assert_eq!(count_paired(&db), 1);
}

