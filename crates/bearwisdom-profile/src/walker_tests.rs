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

#[test]
fn dot_h_with_cpp_template_routes_to_cpp() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("foo.h");
    std::fs::write(
        &path,
        "#pragma once\ntemplate<typename T>\nstruct Box { T value; };\n",
    )
    .unwrap();
    let files = walk_files(dir.path());
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].language_id, "cpp");
}

#[test]
fn dot_h_with_namespace_routes_to_cpp() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("ns.h"),
        "namespace foo {\n  int bar();\n}\n",
    )
    .unwrap();
    let files = walk_files(dir.path());
    assert_eq!(files[0].language_id, "cpp");
}

#[test]
fn dot_h_with_class_decl_routes_to_cpp() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("widget.h"),
        "#pragma once\nclass Widget {\npublic:\n  Widget();\n};\n",
    )
    .unwrap();
    let files = walk_files(dir.path());
    assert_eq!(files[0].language_id, "cpp");
}

#[test]
fn dot_h_with_extern_c_routes_to_cpp() {
    // `extern "C"` only appears in C++ headers (it would not parse as
    // C). Pure-C headers don't need it.
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("api.h"),
        "#pragma once\n#ifdef __cplusplus\nextern \"C\" {\n#endif\nint do_thing(void);\n",
    )
    .unwrap();
    let files = walk_files(dir.path());
    assert_eq!(files[0].language_id, "cpp");
}

#[test]
fn dot_h_with_qt_macro_routes_to_cpp() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("model.h"),
        "#pragma once\nclass Model : public QObject {\n  Q_OBJECT\npublic:\n  Model();\n};\n",
    )
    .unwrap();
    let files = walk_files(dir.path());
    assert_eq!(files[0].language_id, "cpp");
}

#[test]
fn pure_c_dot_h_stays_c() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("util.h"),
        concat!(
            "/*\n",
            " * MIT License — pure C utility header.\n",
            " */\n",
            "#ifndef UTIL_H\n",
            "#define UTIL_H\n",
            "#include <stddef.h>\n",
            "struct buffer { char *data; size_t len; };\n",
            "int util_init(struct buffer *b, size_t cap);\n",
            "void util_free(struct buffer *b);\n",
            "#endif\n",
        ),
    )
    .unwrap();
    let files = walk_files(dir.path());
    assert_eq!(files[0].language_id, "c");
}

#[test]
fn git_repo_walks_checked_in_vendor_and_generated_files() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();

    let vendor = dir.path().join("vendor");
    std::fs::create_dir(&vendor).unwrap();
    std::fs::write(vendor.join("lib.js"), "module.exports = {};").unwrap();

    let dist = dir.path().join("dist");
    std::fs::create_dir(&dist).unwrap();
    std::fs::write(dist.join("app.min.js"), "!function(){}();").unwrap();

    let files = walk_files(dir.path());
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(paths.contains(&"app.ts"), "got: {paths:?}");
    assert!(
        paths.contains(&"vendor/lib.js"),
        "checked-in vendor/ should be walked in a git repo: {paths:?}"
    );
    assert!(
        paths.contains(&"dist/app.min.js"),
        "checked-in dist/*.min.js should be walked in a git repo: {paths:?}"
    );
}

#[test]
fn non_git_project_still_excludes_vendor_and_generated_files() {
    let dir = tempfile::TempDir::new().unwrap();
    // Deliberately no `.git` — this is the non-repo safety-net case.
    std::fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();

    let vendor = dir.path().join("vendor");
    std::fs::create_dir(&vendor).unwrap();
    std::fs::write(vendor.join("lib.js"), "module.exports = {};").unwrap();

    let dist = dir.path().join("dist");
    std::fs::create_dir(&dist).unwrap();
    std::fs::write(dist.join("app.min.js"), "!function(){}();").unwrap();

    let files = walk_files(dir.path());
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert_eq!(paths, vec!["app.ts"], "got: {paths:?}");
}

#[test]
fn dot_git_directory_itself_is_never_walked() {
    let dir = tempfile::TempDir::new().unwrap();
    let git_dir = dir.path().join(".git");
    std::fs::create_dir(&git_dir).unwrap();
    // A `.git` internals file that happens to carry a recognised extension —
    // must never surface as a source file regardless of the git-state gate.
    std::fs::write(git_dir.join("hooks.py"), "# not a real file").unwrap();
    std::fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();

    let files = walk_files(dir.path());
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert_eq!(paths, vec!["app.ts"], "got: {paths:?}");
}

#[test]
fn gitignored_node_modules_still_skipped_in_git_repo() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
    std::fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();

    let nm = dir.path().join("node_modules");
    std::fs::create_dir(&nm).unwrap();
    std::fs::write(nm.join("dep.ts"), "const y = 2;").unwrap();

    let files = walk_files(dir.path());
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert_eq!(
        paths, vec!["app.ts"],
        "gitignored node_modules must stay skipped even though it's now admitted for reclassification: {paths:?}"
    );
}

#[test]
fn dot_c_always_routes_to_c_even_with_cpp_strings() {
    // The disambiguation only kicks in when the detector returns "c"
    // (which happens for `.h`, `.c` files). A `.c` file with the literal
    // string "namespace " inside a comment must still route to C — but
    // by extension `.c` already maps to C without any content probe.
    // This test is here to lock in that we don't accidentally widen the
    // disambiguation rule beyond `.h`.
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("a.c"),
        "/* mentions namespace and class in a comment */\nint main(void) { return 0; }\n",
    )
    .unwrap();
    let files = walk_files(dir.path());
    // `.c` is unambiguous regardless of content.
    assert_eq!(files[0].language_id, "c");
}
