use super::*;
use crate::db::Database;
use crate::indexer::stage_discover::{collect_package_dep_rows, manifest_kind_to_ecosystem};
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
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    let stats = full_index(&mut db, dir.path(), None, None, None).unwrap();

    assert!(stats.file_count >= 1, "No files indexed");
    assert!(
        stats.symbol_count >= 2,
        "Expected at least FooService + DoSomething"
    );
}

#[test]
fn index_produces_qualified_names() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("Api.cs"),
        "namespace Catalog { class CatalogApi { void List() {} } }",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    full_index(&mut db, dir.path(), None, None, None).unwrap();

    let qname: String = db
        .conn()
        .query_row(
            "SELECT qualified_name FROM symbols WHERE name = 'List'",
            [],
            |r| r.get(0),
        )
        .unwrap();
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
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
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
        programs: None,
        project_root: std::path::PathBuf::new(),
        manifests: HashMap::new(),
        by_package,
        workspace_pkg_by_declared_name: HashMap::new(),
        workspace_pkg_paths: HashMap::new(),
        active_ecosystems: Vec::new(),
        active_ecosystems_by_package: HashMap::new(),
        language_presence_by_package: HashMap::new(),
        language_presence: Default::default(),
        plugin_state: Default::default(),
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
    use crate::ecosystem::manifest::ManifestKind;
    assert_eq!(
        manifest_kind_to_ecosystem(ManifestKind::Npm),
        Some("typescript")
    );
    assert_eq!(
        manifest_kind_to_ecosystem(ManifestKind::PyProject),
        Some("python")
    );
    assert_eq!(
        manifest_kind_to_ecosystem(ManifestKind::NuGet),
        Some("dotnet")
    );
    assert_eq!(
        manifest_kind_to_ecosystem(ManifestKind::Cargo),
        Some("rust")
    );
    assert_eq!(manifest_kind_to_ecosystem(ManifestKind::GoMod), Some("go"));
    assert_eq!(
        manifest_kind_to_ecosystem(ManifestKind::Maven),
        Some("java")
    );
    assert_eq!(
        manifest_kind_to_ecosystem(ManifestKind::Gradle),
        Some("java")
    );
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
    )
    .unwrap();
    fs::create_dir_all(root.join("apps/web")).unwrap();
    fs::write(
        root.join("apps/web/package.json"),
        r#"{"name":"web","dependencies":{"react":"18","axios":"1"}}"#,
    )
    .unwrap();
    fs::write(root.join("apps/web/index.ts"), r#"export const x = 1;"#).unwrap();
    fs::create_dir_all(root.join("apps/server")).unwrap();
    fs::write(
        root.join("apps/server/package.json"),
        r#"{"name":"server","dependencies":{"axios":"1","express":"4"}}"#,
    )
    .unwrap();
    fs::write(root.join("apps/server/index.ts"), r#"export const y = 2;"#).unwrap();

    let mut db = Database::open_in_memory().unwrap();
    full_index(&mut db, root, None, None, None).unwrap();

    // Web and server packages must each have their own rows.
    let web_deps: Vec<String> = db
        .conn()
        .prepare(
            "SELECT dep_name FROM package_deps pd
                      JOIN packages p ON p.id = pd.package_id
                      WHERE p.name = 'web' AND pd.ecosystem = 'typescript'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        web_deps.contains(&"react".to_string()),
        "web should declare react, got {web_deps:?}"
    );
    assert!(
        web_deps.contains(&"axios".to_string()),
        "web should declare axios"
    );
    assert!(
        !web_deps.contains(&"express".to_string()),
        "web should NOT declare express"
    );

    let server_deps: Vec<String> = db
        .conn()
        .prepare(
            "SELECT dep_name FROM package_deps pd
                      JOIN packages p ON p.id = pd.package_id
                      WHERE p.name = 'server' AND pd.ecosystem = 'typescript'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        server_deps.contains(&"express".to_string()),
        "server should declare express, got {server_deps:?}"
    );
    assert!(
        server_deps.contains(&"axios".to_string()),
        "server should declare axios"
    );
    assert!(
        !server_deps.contains(&"react".to_string()),
        "server should NOT declare react"
    );

    // Acceptance criteria #5 — "which packages declare axios?" returns both.
    let axios_declarers: Vec<String> = db
        .conn()
        .prepare(
            "SELECT p.name FROM package_deps pd
                      JOIN packages p ON p.id = pd.package_id
                      WHERE pd.dep_name = 'axios'
                      ORDER BY p.name",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(axios_declarers, vec!["server", "web"]);
}

/// Regression: content scanners in full.rs used `&content[..content.len().min(N)]`
/// which panicked when byte N landed inside a multi-byte UTF-8 char.
/// c-redis ships a header with a box-drawing glyph (`─`, 3 bytes) whose
/// second byte sat at offset 4096, panicking the streaming-pipeline writer
/// and stalling the indexer silently.  The vendor-banner and generated-
/// platform-header scanners both cap at a byte index; both must clamp to
/// a char boundary before slicing.
#[test]
fn vendor_scan_survives_multibyte_char_at_boundary() {
    // Pad up to just before byte 4096 with ASCII, then drop a 3-byte char
    // so byte 4096 is inside the char.
    let mut src = String::with_capacity(5000);
    src.push_str("/* Header with a box-drawing glyph at the scan cutoff */\n");
    while src.len() < 4094 {
        src.push('a');
    }
    src.push('─'); // 3 bytes: e2 94 80 — cut at 4096 falls inside
    while src.len() < 5000 {
        src.push('b');
    }
    assert!(src.len() > 4096);
    assert!(!src.is_char_boundary(4096));

    // Both content-scan helpers must accept this without panicking.
    let _ = super::is_c_vendored_file("c", "src/x.c", &src);
    let _ = super::is_generated_platform_header("c", &src);
}

#[test]
fn vendor_scan_on_short_content_does_not_panic() {
    // Content shorter than the 4 KiB cutoff: `.min(len)` returns len, so
    // no clamping needed — but the char-boundary walk must still handle
    // the no-op case.
    let _ = super::is_c_vendored_file("c", "src/x.c", "int main() { return 0; }");
    let _ = super::is_generated_platform_header("c", "int main() { return 0; }");
}

fn mk_pypi_root(module_path: &str) -> crate::ecosystem::externals::ExternalDepRoot {
    crate::ecosystem::externals::ExternalDepRoot {
        module_path: module_path.to_string(),
        version: "unknown".to_string(),
        root: std::path::PathBuf::from(format!("/site-packages/{module_path}")),
        ecosystem: "python",
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn robot_root_selection_matches_declared_and_framework() {
    use std::collections::HashSet;
    let declared: HashSet<String> = ["BuiltIn", "SeleniumLibrary"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let roots = vec![
        mk_pypi_root("SeleniumLibrary"),
        mk_pypi_root("robot"),    // framework package — always included
        mk_pypi_root("requests"), // undeclared — must NOT be selected
    ];
    let selected = super::select_robot_library_roots(&declared, &roots);
    let modules: Vec<&str> = selected.iter().map(|r| r.module_path.as_str()).collect();
    assert!(modules.contains(&"SeleniumLibrary"), "declared lib pulled");
    assert!(modules.contains(&"robot"), "framework package pulled");
    assert!(
        !modules.contains(&"requests"),
        "undeclared package must not be pulled: {modules:?}"
    );
}

#[test]
fn robot_root_selection_is_case_insensitive() {
    use std::collections::HashSet;
    let declared: HashSet<String> = ["seleniumlibrary"].iter().map(|s| s.to_string()).collect();
    let roots = vec![mk_pypi_root("SeleniumLibrary")];
    let selected = super::select_robot_library_roots(&declared, &roots);
    assert_eq!(selected.len(), 1, "case-insensitive name match");
}

#[test]
fn robot_root_selection_ignores_non_python_ecosystems() {
    use std::collections::HashSet;
    let declared: HashSet<String> = ["SeleniumLibrary"].iter().map(|s| s.to_string()).collect();
    let mut npm_root = mk_pypi_root("SeleniumLibrary");
    npm_root.ecosystem = "npm";
    let roots = [npm_root];
    let selected = super::select_robot_library_roots(&declared, &roots);
    assert!(
        selected.is_empty(),
        "only python-tagged roots are robot library targets"
    );
}

// ---------------------------------------------------------------
// Checked-in vendor/generated reclassification
// ---------------------------------------------------------------

// `bearwisdom-profile`'s walker (`exclusions.rs::COMMON_EXCLUDE_DIRS` plus
// every language's `exclude_dirs`, e.g. `javascript.rs` declaring `dist`,
// `node_modules`) drops matching directories before a file ever reaches
// this reclassification pass — unless the project root is a git working
// tree, in which case names the shared `vendored_or_generated::classify`
// detector recognizes are admitted instead of hard-excluded (still subject
// to `.gitignore`). These first two fixtures have no `.git` dir, so `dist/`
// and `vendor/` still collide with the walker-level exclusion; they use
// `generated/` and `third_party/`, which the walker never excludes, to
// isolate the reclassification pass itself. The git-repo fixtures below
// exercise the now-reachable `vendor/`/`dist/` paths end to end.

#[test]
fn checked_in_generated_file_reclassifies_while_near_miss_stays_internal() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("generated")).unwrap();
    fs::write(
        root.join("generated/client.pb.go"),
        "package generated\n\nfunc vendoredHelper() int { return 1 }\n",
    )
    .unwrap();

    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/build_tools.rs"),
        "pub fn build_tools_helper() -> i32 { 1 }\n",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    full_index(&mut db, root, None, None, None).unwrap();
    let conn = db.conn();

    // The checked-in codegen file is reclassified external under the new
    // `ext:generated:` tag.
    let (generated_path, generated_origin): (String, String) = conn
        .query_row(
            "SELECT path, origin FROM files WHERE path LIKE 'ext:generated:%'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("generated/client.pb.go should be reclassified");
    assert!(generated_path.starts_with("ext:generated:"));
    assert!(generated_path.ends_with("client.pb.go"));
    assert_eq!(generated_origin, "external");

    // `src/build_tools.rs` is a near-miss for the `build` segment (the
    // filename, not a path segment, contains "build") — it must stay
    // internal and unprefixed.
    let build_tools_origin: String = conn
        .query_row(
            "SELECT origin FROM files WHERE path = 'src/build_tools.rs'",
            [],
            |r| r.get(0),
        )
        .expect("src/build_tools.rs should be indexed as internal");
    assert_eq!(build_tools_origin, "internal");
}

#[test]
fn reclassified_file_symbol_remains_a_lookup_target() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("third_party")).unwrap();
    fs::write(
        root.join("third_party/helper.js"),
        "function vendoredHelper() { return 1; }\n",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    full_index(&mut db, root, None, None, None).unwrap();

    // The vendored file's own symbol is still written — external-origin,
    // available to the resolver as a lookup target, not silently dropped.
    let symbol_origin: String = db
        .conn()
        .query_row(
            "SELECT origin FROM symbols WHERE name = 'vendoredHelper'",
            [],
            |r| r.get(0),
        )
        .expect("vendoredHelper symbol should still be indexed");
    assert_eq!(symbol_origin, "external");
}

#[test]
fn git_repo_checked_in_vendor_dir_reclassifies() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".git")).unwrap();

    fs::create_dir_all(root.join("vendor")).unwrap();
    fs::write(
        root.join("vendor/lib.js"),
        "function checkedInVendorHelper() { return 1; }\n",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    full_index(&mut db, root, None, None, None).unwrap();
    let conn = db.conn();

    let (vendor_path, vendor_origin): (String, String) = conn
        .query_row(
            "SELECT path, origin FROM files WHERE path LIKE 'ext:vendored:%'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("checked-in vendor/lib.js should be reclassified in a git repo");
    assert!(vendor_path.ends_with("vendor/lib.js"));
    assert_eq!(vendor_origin, "external");

    let symbol_origin: String = conn
        .query_row(
            "SELECT origin FROM symbols WHERE name = 'checkedInVendorHelper'",
            [],
            |r| r.get(0),
        )
        .expect("checkedInVendorHelper symbol should still be indexed");
    assert_eq!(symbol_origin, "external");
}

#[test]
fn git_repo_checked_in_minified_dist_file_reclassifies() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".git")).unwrap();

    fs::create_dir_all(root.join("dist")).unwrap();
    fs::write(
        root.join("dist/app.min.js"),
        "function checkedInBundleHelper() { return 1; }\n",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    full_index(&mut db, root, None, None, None).unwrap();
    let conn = db.conn();

    let (generated_path, generated_origin): (String, String) = conn
        .query_row(
            "SELECT path, origin FROM files WHERE path LIKE 'ext:generated:%'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("checked-in dist/app.min.js should be reclassified in a git repo");
    assert!(generated_path.ends_with("dist/app.min.js"));
    assert_eq!(generated_origin, "external");
}

#[test]
fn non_git_project_still_excludes_vendor_dir_at_the_walker() {
    // Same fixture as `git_repo_checked_in_vendor_dir_reclassifies`, minus
    // the `.git` dir — the safety-net case. `vendor/` never reaches the
    // reclassification pass because the walker hard-excludes it first.
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    fs::create_dir_all(root.join("vendor")).unwrap();
    fs::write(
        root.join("vendor/lib.js"),
        "function checkedInVendorHelper() { return 1; }\n",
    )
    .unwrap();

    let mut db = Database::open_in_memory().unwrap();
    full_index(&mut db, root, None, None, None).unwrap();
    let conn = db.conn();

    let file_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path LIKE '%vendor%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        file_count, 0,
        "vendor/ must not be walked without a .git dir"
    );
}
