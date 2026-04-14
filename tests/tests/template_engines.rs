//! Integration tests for E8 — Node/JS template engines.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn count_js_symbols(db: &bearwisdom::Database, like: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE ?1
           AND s.origin_language = 'javascript'",
        rusqlite::params![like],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn handlebars_expression_dispatches_to_javascript() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("list.hbs"),
        "<ul>{{#each items}}<li>{{ renderItem(this) }}</li>{{/each}}</ul>",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%list.hbs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "handlebars");
    // renderItem becomes a JS-origin call ref on a JS-origin wrapper symbol.
    assert!(count_js_symbols(&db, "%list.hbs") >= 1);
}

#[test]
fn pug_dash_code_line_dispatches_to_javascript() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.pug"),
        "- const user = loadUser()\nh1= user.name\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%page.pug'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "pug");
    assert!(count_js_symbols(&db, "%page.pug") >= 1);
}

#[test]
fn ejs_expression_dispatches_to_javascript() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("index.ejs"),
        "<p>Hello <%= getUserName() %></p>\n<% const x = compute(); %>\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%index.ejs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "ejs");
    assert!(count_js_symbols(&db, "%index.ejs") >= 1);
}

#[test]
fn nunjucks_extends_produces_imports_ref() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.njk"),
        "{% extends \"base.njk\" %}\n{% block content %}hi{% endblock %}\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%page.njk'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "nunjucks");

    let imports: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.njk'
               AND ur.kind = 'imports'
               AND ur.target_name = 'base'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(imports >= 1);
}
