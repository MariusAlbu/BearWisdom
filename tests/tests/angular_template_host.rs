//! Integration tests for E7 — Angular component templates.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

#[test]
fn component_html_detected_as_angular_template() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("user.component.html"),
        r#"<div>
<app-avatar [src]="user.avatarUrl" />
<button (click)="onEdit(user.id)">Edit</button>
<ul>
  <li *ngFor="let u of users">{{ u.name }}</li>
</ul>
</div>
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%user.component.html'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "angular_template");
}

#[test]
fn child_component_tags_produce_calls_refs() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.component.html"),
        "<app-avatar></app-avatar>\n<UserCard />\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let targets: Vec<String> = db
        .prepare(
            "SELECT DISTINCT ur.target_name FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.component.html'
               AND ur.kind = 'calls'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(targets.iter().any(|n| n == "AppAvatar"), "got {targets:?}");
    assert!(targets.iter().any(|n| n == "UserCard"), "got {targets:?}");
}

#[test]
fn binding_expression_identifiers_surface_as_ts_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("form.component.html"),
        r#"<input [value]="formName" (input)="handleInput($event)" />"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // Each binding expression becomes a TS StringDsl region. The
    // identifiers (formName, handleInput) land as unresolved refs
    // attributed to the file's host symbol.
    // Event handler call expressions surface as unresolved refs.
    // (Bare-identifier property bindings like `[value]="formName"`
    // don't produce refs because the TS extractor tracks edges — call
    // expressions, type refs — not every identifier. That's a planned
    // future enhancement.)
    let targets: Vec<String> = db
        .prepare(
            "SELECT DISTINCT ur.target_name FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%form.component.html'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        targets.iter().any(|n| n == "handleInput"),
        "expected 'handleInput' from (input) event-binding call, got {targets:?}"
    );
}
