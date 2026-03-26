use super::*;

#[test]
fn walk_finds_source_files() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
    std::fs::write(dir.path().join("lib.ts"), "export const x = 1;").unwrap();
    std::fs::write(dir.path().join("image.png"), "binary").unwrap();

    let files = walk_files(dir.path());
    assert_eq!(files.len(), 2); // .rs and .ts, not .png
    assert!(files.iter().any(|f| f.language_id == "rust"));
    assert!(files.iter().any(|f| f.language_id == "typescript"));
}

#[test]
fn walk_excludes_node_modules() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();
    let nm = dir.path().join("node_modules");
    std::fs::create_dir(&nm).unwrap();
    std::fs::write(nm.join("dep.ts"), "const y = 2;").unwrap();

    let files = walk_files(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].relative_path, "app.ts");
}

#[test]
fn walk_results_are_sorted() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("z.rs"), "").unwrap();
    std::fs::write(dir.path().join("a.rs"), "").unwrap();
    std::fs::write(dir.path().join("m.rs"), "").unwrap();

    let files = walk_files(dir.path());
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}

#[test]
fn walk_normalizes_paths_to_forward_slashes() {
    let dir = tempfile::TempDir::new().unwrap();
    let sub = dir.path().join("src");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("lib.rs"), "").unwrap();

    let files = walk_files(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].relative_path, "src/lib.rs");
}
