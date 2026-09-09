use super::*;

#[test]
fn replacement_is_file_scoped_and_deletion_cascades_by_id() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
        CREATE TABLE symbols (id INTEGER PRIMARY KEY, file_id INTEGER);
        INSERT INTO symbols VALUES (1,10),(2,10),(3,20);",
    )
    .unwrap();
    create(&conn).unwrap();
    create(&conn).unwrap();
    replace(&conn, 10, [1, 2]).unwrap();
    replace(&conn, 20, [3]).unwrap();
    replace(&conn, 10, [2]).unwrap();
    let mut rows = read(&conn).unwrap();
    rows.sort();
    assert_eq!(rows, [2, 3]);
    conn.execute("DELETE FROM symbols WHERE id=2", []).unwrap();
    assert_eq!(read(&conn).unwrap(), [3]);
    assert!(
        replace(&conn, 10, [99]).is_err(),
        "missing identity cannot be persisted"
    );
}
