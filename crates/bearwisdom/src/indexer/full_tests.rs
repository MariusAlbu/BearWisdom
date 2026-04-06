    use super::*;
    use crate::db::Database;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn index_simple_csharp_project() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Foo.cs"),
            r#"
namespace App {
    public class FooService {
        public void DoSomething() {}
    }
}
"#,
        ).unwrap();

        let mut db = Database::open_in_memory().unwrap();
        let stats = full_index(&mut db, dir.path(), None, None, None).unwrap();

        assert!(stats.file_count >= 1, "No files indexed");
        assert!(stats.symbol_count >= 2, "Expected at least FooService + DoSomething");
    }

    #[test]
    fn index_produces_qualified_names() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Api.cs"),
            "namespace Catalog { class CatalogApi { void List() {} } }",
        ).unwrap();

        let mut db = Database::open_in_memory().unwrap();
        full_index(&mut db, dir.path(), None, None, None).unwrap();

        let qname: String = db.conn.query_row(
            "SELECT qualified_name FROM symbols WHERE name = 'List'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(qname, "Catalog.CatalogApi.List");
    }

    #[test]
    fn index_empty_directory_produces_zero_stats() {
        let dir = TempDir::new().unwrap();
        let mut db = Database::open_in_memory().unwrap();
        let stats = full_index(&mut db, dir.path(), None, None, None).unwrap();
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.symbol_count, 0);
    }
