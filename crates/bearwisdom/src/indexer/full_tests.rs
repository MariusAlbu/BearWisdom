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

        let qname: String = db.conn().query_row(
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

    // ---------------------------------------------------------------
    // M3 — per-package locator scoping
    // ---------------------------------------------------------------

    #[test]
    fn m3_collect_package_dep_rows_skips_empty_context() {
        let ctx = super::super::project_context::ProjectContext::default();
        let rows = collect_package_dep_rows(&ctx);
        assert!(rows.is_empty(), "empty context should yield no rows");
    }

    #[test]
    fn m3_collect_package_dep_rows_emits_one_row_per_declared_dep() {
        use crate::indexer::manifest::{ManifestData, ManifestKind};
        use std::collections::{HashMap, HashSet};

        let mut by_package: HashMap<i64, HashMap<ManifestKind, ManifestData>> = HashMap::new();
        let mut pkg1_manifests = HashMap::new();
        let mut pkg1_data = ManifestData::default();
        pkg1_data.dependencies = HashSet::from(["react".to_string(), "axios".to_string()]);
        pkg1_manifests.insert(ManifestKind::Npm, pkg1_data);
        by_package.insert(1, pkg1_manifests);

        let mut pkg2_manifests = HashMap::new();
        let mut pkg2_data = ManifestData::default();
        pkg2_data.dependencies = HashSet::from(["fastapi".to_string()]);
        pkg2_manifests.insert(ManifestKind::PyProject, pkg2_data);
        by_package.insert(2, pkg2_manifests);

        let ctx = super::super::project_context::ProjectContext {
            manifests: HashMap::new(),
            by_package,
            workspace_pkg_by_declared_name: HashMap::new(),
            workspace_pkg_paths: HashMap::new(),
            gradle_catalog_names: Vec::new(),
            active_ecosystems: Vec::new(),
            language_presence: Default::default(),
        };
        let rows = collect_package_dep_rows(&ctx);
        assert_eq!(rows.len(), 3, "expected 3 dep rows, got {rows:?}");

        let pkg1_rows: Vec<_> = rows.iter().filter(|(id, ..)| *id == 1).collect();
        assert_eq!(pkg1_rows.len(), 2);
        assert!(pkg1_rows.iter().all(|(_, eco, ..)| *eco == "typescript"));

        let pkg2_rows: Vec<_> = rows.iter().filter(|(id, ..)| *id == 2).collect();
        assert_eq!(pkg2_rows.len(), 1);
        assert_eq!(pkg2_rows[0].1, "python");
        assert_eq!(pkg2_rows[0].2, "fastapi");
    }

    #[test]
    fn m3_manifest_kind_to_ecosystem_covers_common_kinds() {
        use crate::indexer::manifest::ManifestKind;
        assert_eq!(manifest_kind_to_ecosystem(ManifestKind::Npm), Some("typescript"));
        assert_eq!(manifest_kind_to_ecosystem(ManifestKind::PyProject), Some("python"));
        assert_eq!(manifest_kind_to_ecosystem(ManifestKind::NuGet), Some("dotnet"));
        assert_eq!(manifest_kind_to_ecosystem(ManifestKind::Cargo), Some("rust"));
        assert_eq!(manifest_kind_to_ecosystem(ManifestKind::GoMod), Some("go"));
        assert_eq!(manifest_kind_to_ecosystem(ManifestKind::Maven), Some("java"));
        assert_eq!(manifest_kind_to_ecosystem(ManifestKind::Gradle), Some("java"));
    }

    #[test]
    fn m3_package_deps_written_for_monorepo() {
        // Real end-to-end: two workspace packages with distinct manifests.
        // The index should populate `package_deps` with one row per declared
        // dep per package. Does NOT require node_modules to be present —
        // `package_deps` is derived from manifests, not filesystem probes.
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(
            root.join("package.json"),
            r#"{"name":"ws","private":true,"workspaces":["apps/web","apps/server"]}"#,
        ).unwrap();
        fs::create_dir_all(root.join("apps/web")).unwrap();
        fs::write(
            root.join("apps/web/package.json"),
            r#"{"name":"web","dependencies":{"react":"18","axios":"1"}}"#,
        ).unwrap();
        fs::write(
            root.join("apps/web/index.ts"),
            r#"export const x = 1;"#,
        ).unwrap();
        fs::create_dir_all(root.join("apps/server")).unwrap();
        fs::write(
            root.join("apps/server/package.json"),
            r#"{"name":"server","dependencies":{"axios":"1","express":"4"}}"#,
        ).unwrap();
        fs::write(
            root.join("apps/server/index.ts"),
            r#"export const y = 2;"#,
        ).unwrap();

        let mut db = Database::open_in_memory().unwrap();
        full_index(&mut db, root, None, None, None).unwrap();

        // Web and server packages must each have their own rows.
        let web_deps: Vec<String> = db.conn()
            .prepare("SELECT dep_name FROM package_deps pd
                      JOIN packages p ON p.id = pd.package_id
                      WHERE p.name = 'web' AND pd.ecosystem = 'typescript'")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0)).unwrap()
            .flatten().collect();
        assert!(web_deps.contains(&"react".to_string()), "web should declare react, got {web_deps:?}");
        assert!(web_deps.contains(&"axios".to_string()), "web should declare axios");
        assert!(!web_deps.contains(&"express".to_string()), "web should NOT declare express");

        let server_deps: Vec<String> = db.conn()
            .prepare("SELECT dep_name FROM package_deps pd
                      JOIN packages p ON p.id = pd.package_id
                      WHERE p.name = 'server' AND pd.ecosystem = 'typescript'")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0)).unwrap()
            .flatten().collect();
        assert!(server_deps.contains(&"express".to_string()), "server should declare express, got {server_deps:?}");
        assert!(server_deps.contains(&"axios".to_string()), "server should declare axios");
        assert!(!server_deps.contains(&"react".to_string()), "server should NOT declare react");

        // Acceptance criteria #5 — "which packages declare axios?" returns both.
        let axios_declarers: Vec<String> = db.conn()
            .prepare("SELECT p.name FROM package_deps pd
                      JOIN packages p ON p.id = pd.package_id
                      WHERE pd.dep_name = 'axios'
                      ORDER BY p.name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0)).unwrap()
            .flatten().collect();
        assert_eq!(axios_declarers, vec!["server", "web"]);
    }
