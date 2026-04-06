use super::*;

#[test]
fn pluralise_default() {
    assert_eq!(pluralise("CatalogItem"), "CatalogItems");
    assert_eq!(pluralise("Order"), "Orders");
}

#[test]
fn pluralise_y_ending() {
    assert_eq!(pluralise("Category"), "Categories");
    assert_eq!(pluralise("Country"), "Countries");
}

#[test]
fn pluralise_vowel_y_unchanged() {
    // "key" ends in "ey" — Rule 2 doesn't apply, Rule 1 doesn't apply → "keys"
    assert_eq!(pluralise("Key"), "Keys");
}

#[test]
fn pluralise_sibilant() {
    assert_eq!(pluralise("Address"), "Addresses");
    assert_eq!(pluralise("Tax"), "Taxes");
}

#[test]
fn connect_runs_on_empty_db() {
    let db = Database::open_in_memory().unwrap();
    connect(&db).unwrap();
}

#[test]
fn write_and_list_db_mapping() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    // Set up a minimal file + symbol.
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES ('db.cs', 'h', 'csharp', 0)",
        [],
    ).unwrap();
    let file_id: i64 = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'Items', 'CatalogDbContext.Items', 'property', 5, 0)",
        [file_id],
    ).unwrap();
    let sym_id: i64 = conn.last_insert_rowid();

    write_db_mapping(conn, sym_id, "CatalogItem", "CatalogItem", DbMappingSource::Convention).unwrap();
    connect(&db).unwrap();

    let mappings = list_mappings(&db).unwrap();
    assert_eq!(mappings.len(), 1);
    // After pluralisation, the convention table name should be "CatalogItems".
    assert_eq!(mappings[0].table_name, "CatalogItems");
    assert_eq!(mappings[0].entity_type, "CatalogItem");
}
