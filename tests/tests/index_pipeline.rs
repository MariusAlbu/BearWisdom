//! Integration tests for the full indexing pipeline.
//!
//! Exercises: walk → full_index → verify stats/symbols/edges in the database.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;

#[test]
fn index_csharp_project() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();

    let stats = full_index(&mut db, project.path(), None, None).unwrap();

    assert!(stats.file_count >= 4, "expected at least 4 C# files, got {}", stats.file_count);
    assert!(stats.symbol_count > 0, "expected symbols from C# extraction");
    assert_eq!(stats.files_with_errors, 0, "no files should have parse errors");
}

#[test]
fn index_python_project() {
    let project = TestProject::python_app();
    let mut db = TestProject::in_memory_db();

    let stats = full_index(&mut db, project.path(), None, None).unwrap();

    assert!(stats.file_count >= 2, "expected at least 2 Python files, got {}", stats.file_count);
    assert!(stats.symbol_count > 0, "expected symbols from Python extraction");
}

#[test]
fn index_typescript_project() {
    let project = TestProject::typescript_app();
    let mut db = TestProject::in_memory_db();

    let stats = full_index(&mut db, project.path(), None, None).unwrap();

    assert!(stats.file_count >= 2, "expected at least 2 TypeScript files, got {}", stats.file_count);
    assert!(stats.symbol_count > 0, "expected symbols from TypeScript extraction");
}

#[test]
fn index_multi_language_project() {
    let project = TestProject::multi_lang();
    let mut db = TestProject::in_memory_db();

    let stats = full_index(&mut db, project.path(), None, None).unwrap();

    assert!(stats.file_count >= 3, "expected files from 3 languages");
    assert!(stats.symbol_count > 0);
}

#[test]
fn index_empty_directory() {
    let project = TestProject { dir: tempfile::TempDir::new().unwrap() };
    let mut db = TestProject::in_memory_db();

    let stats = full_index(&mut db, project.path(), None, None).unwrap();

    assert_eq!(stats.file_count, 0);
    assert_eq!(stats.symbol_count, 0);
    assert_eq!(stats.edge_count, 0);
}

#[test]
fn index_with_progress_callback() {
    use std::sync::{Arc, Mutex};

    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();

    let steps: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let steps_clone = steps.clone();
    let progress: bearwisdom::ProgressFn = Box::new(move |step, _pct, _detail| {
        steps_clone.lock().unwrap().push(step.to_string());
    });

    let stats = full_index(&mut db, project.path(), Some(progress), None).unwrap();
    assert!(stats.symbol_count > 0);

    let captured = steps.lock().unwrap();
    assert!(!captured.is_empty(), "progress callback should have been invoked");
}

#[test]
fn index_produces_edges() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();

    let stats = full_index(&mut db, project.path(), None, None).unwrap();

    // The C# fixture has implements (ProductRepository : IProductRepository),
    // type_ref / calls edges, and import references.
    assert!(stats.edge_count > 0, "expected edges from C# relationships, got 0");
}

#[test]
fn index_with_pre_walked_files() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();

    let files = bearwisdom::walker::walk(project.path()).unwrap();
    assert!(!files.is_empty());

    let stats = full_index(&mut db, project.path(), None, Some(files)).unwrap();
    assert!(stats.file_count > 0);
    assert!(stats.symbol_count > 0);
}
