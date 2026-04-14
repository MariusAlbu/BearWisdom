//! Integration tests for incremental indexing.
//!
//! Exercises: full_index → mutate files → incremental_index → verify updates.

use std::fs;

use bearwisdom::query::architecture::get_overview;
use bearwisdom::{full_index, incremental_index};
use bearwisdom_tests::TestProject;

#[test]
fn incremental_detects_new_file() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();

    let stats1 = full_index(&mut db, project.path(), None, None, None).unwrap();
    let count_before = stats1.symbol_count;

    // Add a new file after the initial index.
    project.add_file("Services/OrderService.cs", r#"
namespace MyApp.Services
{
    public class OrderService
    {
        public void PlaceOrder(int productId) { }
    }
}
"#);

    let stats2 = incremental_index(&mut db, project.path(), None).unwrap();

    // The new file should contribute at least one new symbol.
    let overview = get_overview(&db).unwrap();
    assert!(
        overview.total_symbols > count_before,
        "expected more symbols after adding OrderService.cs: before={count_before}, after={}",
        overview.total_symbols,
    );
    let _ = stats2;
}

#[test]
fn incremental_detects_modified_file() {
    let project = TestProject::python_app();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // Modify an existing file — add a new class.
    let path = project.path().join("models.py");
    let mut content = fs::read_to_string(&path).unwrap();
    content.push_str(r#"

class Bird(Animal):
    def speak(self) -> str:
        return f"{self.name} says Tweet!"
"#);
    fs::write(&path, content).unwrap();

    let stats = incremental_index(&mut db, project.path(), None).unwrap();
    let _ = stats;

    // "Bird" should now be findable.
    let results = bearwisdom::query::search::search_symbols(&db, "Bird", 10, &bearwisdom::query::QueryOptions::full()).unwrap();
    assert!(!results.is_empty(), "Bird class should be indexed after incremental update");
}

#[test]
fn incremental_detects_deleted_file() {
    let project = TestProject::typescript_app();
    let mut db = TestProject::in_memory_db();

    let stats1 = full_index(&mut db, project.path(), None, None, None).unwrap();
    let files_before = stats1.file_count;

    // Delete one of the two files.
    let target = project.path().join("types.ts");
    fs::remove_file(&target).unwrap();

    let stats2 = incremental_index(&mut db, project.path(), None).unwrap();
    let _ = stats2;

    let overview = get_overview(&db).unwrap();
    assert!(
        overview.total_files < files_before,
        "file count should decrease after deletion: before={files_before}, after={}",
        overview.total_files,
    );
}

#[test]
fn git_reindex_falls_back_to_hash_diff_when_not_a_git_repo() {
    // The `bw open` idempotency path calls git_reindex on an existing DB.
    // For non-git projects (TestProject fixtures don't initialize a repo),
    // git_reindex must fall back to HashDiff internally — not panic or
    // error. Regression guard for the CLI wire-up that assumed git when
    // `indexed_commit` happened to be set.
    use bearwisdom::git_reindex;

    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    // No changes, no git — should not error.
    let stats = git_reindex(&mut db, project.path(), None)
        .expect("git_reindex should fall back cleanly on non-git projects");
    // Nothing changed on disk since the last full index.
    assert_eq!(stats.files_added, 0);
    assert_eq!(stats.files_modified, 0);
    assert_eq!(stats.files_deleted, 0);
}

#[test]
fn incremental_on_unchanged_project_is_noop() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();

    full_index(&mut db, project.path(), None, None, None).unwrap();

    let overview_before = get_overview(&db).unwrap();
    let syms_before = overview_before.total_symbols;

    // Run incremental with no changes.
    incremental_index(&mut db, project.path(), None).unwrap();

    let overview_after = get_overview(&db).unwrap();
    assert_eq!(
        syms_before, overview_after.total_symbols,
        "symbol count should not change when nothing was modified"
    );
}
