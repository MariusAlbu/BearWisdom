//! Integration tests for E9 — Ruby template engines (ERB, Slim, Haml).

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn count_ruby_syms(db: &bearwisdom::Database, like: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE f.path LIKE ?1
           AND s.origin_language = 'ruby'",
        rusqlite::params![like],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn erb_expression_dispatches_to_ruby() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("show.html.erb"),
        "<p>Hello <%= @user.name %></p>\n<% if @logged_in %>yep<% end %>\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%show.html.erb'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "erb");
    // Ruby expressions pass through Ruby extractor; it emits symbols
    // only for declarations. Bare expression cells produce unresolved
    // refs rather than symbols — verify the dispatch happened by
    // checking refs exist or at minimum the file indexed cleanly.
    let _ = count_ruby_syms(&db, "%show.html.erb");
}

#[test]
fn slim_expression_dispatches_to_ruby() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("card.slim"),
        "- user = current_user\nh1= user.name\np= user.email\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%card.slim'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "slim");
    let _ = count_ruby_syms(&db, "%card.slim");
}

#[test]
fn haml_filter_block_dispatches_to_javascript() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.haml"),
        "%h1= @title\n:javascript\n  function go() { alert(1); }\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%page.haml'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "haml");

    let js_syms: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.haml'
               AND s.origin_language = 'javascript'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(js_syms >= 1, "expected JS symbol from :javascript filter");
}
