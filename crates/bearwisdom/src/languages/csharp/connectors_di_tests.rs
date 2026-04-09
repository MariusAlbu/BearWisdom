use super::*;
use crate::db::Database;

// -----------------------------------------------------------------------
// Unit tests for source detection
// -----------------------------------------------------------------------

#[test]
fn detects_two_type_scoped() {
    let re_two = build_two_type_regex();
    let re_one = build_one_type_regex();
    let mut out = Vec::new();
    detect_in_source(
        r#"services.AddScoped<ICatalogService, CatalogService>();"#,
        1,
        &re_two,
        &re_one,
        &mut out,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].lifetime, "scoped");
    assert_eq!(out[0].interface_type, "ICatalogService");
    assert_eq!(out[0].implementation_type, "CatalogService");
}

#[test]
fn detects_two_type_transient() {
    let re_two = build_two_type_regex();
    let re_one = build_one_type_regex();
    let mut out = Vec::new();
    detect_in_source(
        r#"services.AddTransient<IOrderService, OrderService>();"#,
        2,
        &re_two,
        &re_one,
        &mut out,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].lifetime, "transient");
    assert_eq!(out[0].interface_type, "IOrderService");
    assert_eq!(out[0].implementation_type, "OrderService");
}

#[test]
fn detects_two_type_singleton() {
    let re_two = build_two_type_regex();
    let re_one = build_one_type_regex();
    let mut out = Vec::new();
    detect_in_source(
        r#"services.AddSingleton<ICache, RedisCache>();"#,
        3,
        &re_two,
        &re_one,
        &mut out,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].lifetime, "singleton");
    assert_eq!(out[0].interface_type, "ICache");
    assert_eq!(out[0].implementation_type, "RedisCache");
}

#[test]
fn detects_one_type_form() {
    let re_two = build_two_type_regex();
    let re_one = build_one_type_regex();
    let mut out = Vec::new();
    detect_in_source(
        r#"services.AddScoped<CatalogService>();"#,
        5,
        &re_two,
        &re_one,
        &mut out,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].interface_type, "CatalogService");
    assert_eq!(out[0].implementation_type, "CatalogService");
}

#[test]
fn two_type_takes_priority_over_one_type() {
    let re_two = build_two_type_regex();
    let re_one = build_one_type_regex();
    let mut out = Vec::new();
    detect_in_source(
        r#"services.AddScoped<IFoo, Foo>();"#,
        1,
        &re_two,
        &re_one,
        &mut out,
    );
    assert_eq!(out.len(), 1, "Two-type match should not also emit a one-type match");
    assert_eq!(out[0].interface_type, "IFoo");
}

#[test]
fn empty_source_produces_no_registrations() {
    let re_two = build_two_type_regex();
    let re_one = build_one_type_regex();
    let mut out = Vec::new();
    detect_in_source("// no registrations here", 1, &re_two, &re_one, &mut out);
    assert!(out.is_empty());
}

// -----------------------------------------------------------------------
// Integration tests against in-memory DB
// -----------------------------------------------------------------------

fn seed_symbols(db: &Database) -> (i64, i64) {
    let conn = db.conn();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('Services.cs', 'h1', 'csharp', 0)",
        [],
    )
    .unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'ICatalogService', 'App.ICatalogService', 'interface', 5, 0)",
        [file_id],
    )
    .unwrap();
    let iface_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'CatalogService', 'App.CatalogService', 'class', 20, 0)",
        [file_id],
    )
    .unwrap();
    let impl_id: i64 = conn.last_insert_rowid();

    (iface_id, impl_id)
}

#[test]
fn link_creates_implements_edge() {
    let db = Database::open_in_memory().unwrap();
    let (_, impl_id) = seed_symbols(&db);

    let file_id: i64 = db
        .conn()
        .query_row(
            "SELECT file_id FROM symbols WHERE id = ?1",
            [impl_id],
            |r| r.get(0),
        )
        .unwrap();

    let registrations = vec![DiRegistration {
        file_id,
        line: 42,
        lifetime: "scoped".to_string(),
        interface_type: "ICatalogService".to_string(),
        implementation_type: "CatalogService".to_string(),
    }];

    let created = link_di_registrations(db.conn(), &registrations).unwrap();
    assert_eq!(created, 1, "Expected one implements edge");

    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE kind = 'implements'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn self_registration_creates_no_edge() {
    let db = Database::open_in_memory().unwrap();
    seed_symbols(&db);

    let registrations = vec![DiRegistration {
        file_id: 1,
        line: 10,
        lifetime: "scoped".to_string(),
        interface_type: "CatalogService".to_string(),
        implementation_type: "CatalogService".to_string(),
    }];

    let created = link_di_registrations(db.conn(), &registrations).unwrap();
    assert_eq!(created, 0, "Self-registration should produce no edge");
}

#[test]
fn missing_symbols_skipped_without_error() {
    let db = Database::open_in_memory().unwrap();

    let registrations = vec![DiRegistration {
        file_id: 1,
        line: 5,
        lifetime: "transient".to_string(),
        interface_type: "INonExistent".to_string(),
        implementation_type: "AlsoNonExistent".to_string(),
    }];

    let created = link_di_registrations(db.conn(), &registrations).unwrap();
    assert_eq!(created, 0);
}

#[test]
fn duplicate_registration_not_double_counted() {
    let db = Database::open_in_memory().unwrap();
    let (_, impl_id) = seed_symbols(&db);

    let file_id: i64 = db
        .conn()
        .query_row(
            "SELECT file_id FROM symbols WHERE id = ?1",
            [impl_id],
            |r| r.get(0),
        )
        .unwrap();

    let reg = DiRegistration {
        file_id,
        line: 42,
        lifetime: "scoped".to_string(),
        interface_type: "ICatalogService".to_string(),
        implementation_type: "CatalogService".to_string(),
    };

    link_di_registrations(db.conn(), &[reg.clone()]).unwrap();
    let created = link_di_registrations(db.conn(), &[reg]).unwrap();
    assert_eq!(created, 0, "OR IGNORE should prevent duplicate edge");
}
