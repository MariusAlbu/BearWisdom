use super::*;
use crate::db::Database;

// -----------------------------------------------------------------------
// Unit tests for detection helpers
// -----------------------------------------------------------------------

#[test]
fn store_regex_matches_export_const() {
    let re = build_store_regex();
    let line = "export const useEditorStore = create<EditorState>((set) => ({";
    assert!(re.is_match(line));
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "useEditorStore");
}

#[test]
fn store_regex_matches_const_without_export() {
    let re = build_store_regex();
    let line = "const useAuthStore = create(initializer)";
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "useAuthStore");
}

#[test]
fn store_regex_does_not_match_non_use_prefix() {
    let re = build_store_regex();
    // Variable not named use* — should not match.
    assert!(!re.is_match("const myState = create<State>()"));
}

#[test]
fn extract_component_name_from_meta_type() {
    let re_default = build_default_export_regex();
    let re_meta = build_meta_type_regex();
    let source = "const meta: Meta<typeof Button> = { title: 'Button' };\nexport default meta;";
    let name = extract_component_name(source, &re_default, &re_meta);
    assert_eq!(name, Some("Button".to_string()));
}

#[test]
fn extract_component_name_from_default_export() {
    let re_default = build_default_export_regex();
    let re_meta = build_meta_type_regex();
    let source = "export default { component: FileTree, title: 'FileTree' };";
    let name = extract_component_name(source, &re_default, &re_meta);
    assert_eq!(name, Some("FileTree".to_string()));
}

#[test]
fn extract_component_name_meta_takes_priority() {
    // Both patterns present — Meta should win (scanned first).
    let re_default = build_default_export_regex();
    let re_meta = build_meta_type_regex();
    let source = "const meta: Meta<typeof Button> = { component: OtherThing };";
    let name = extract_component_name(source, &re_default, &re_meta);
    assert_eq!(name, Some("Button".to_string()));
}

#[test]
fn extract_component_name_returns_none_when_no_match() {
    let re_default = build_default_export_regex();
    let re_meta = build_meta_type_regex();
    let source = "// no story metadata here";
    assert!(extract_component_name(source, &re_default, &re_meta).is_none());
}

// -----------------------------------------------------------------------
// Integration tests
// -----------------------------------------------------------------------

fn seed_store_symbol(db: &Database) -> i64 {
    let conn = &db.conn;

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/stores/editorStore.ts', 'h1', 'typescript', 0)",
        [],
    )
    .unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'useEditorStore', 'useEditorStore', 'variable', 3, 0)",
        [file_id],
    )
    .unwrap();
    let sym_id: i64 = conn.last_insert_rowid();

    // Store the file_id in the ZustandStore via the file_id field.
    let _ = file_id;
    sym_id
}

#[test]
fn create_react_concepts_adds_zustand_concept() {
    let db = Database::open_in_memory().unwrap();
    let sym_id = seed_store_symbol(&db);

    // Get file_id from the symbol.
    let file_id: i64 = db
        .conn
        .query_row("SELECT file_id FROM symbols WHERE id = ?1", [sym_id], |r| r.get(0))
        .unwrap();

    let stores = vec![ZustandStore {
        file_id,
        name: "useEditorStore".to_string(),
        line: 3,
    }];

    create_react_concepts(&db.conn, &stores, &[]).unwrap();

    let concept_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM concepts WHERE name = 'zustand-stores'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(concept_count, 1, "zustand-stores concept should be created");

    let member_count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM concept_members WHERE auto_assigned = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(member_count, 1, "Store symbol should be added as member");
}

#[test]
fn create_react_concepts_idempotent() {
    let db = Database::open_in_memory().unwrap();
    let sym_id = seed_store_symbol(&db);

    let file_id: i64 = db
        .conn
        .query_row("SELECT file_id FROM symbols WHERE id = ?1", [sym_id], |r| r.get(0))
        .unwrap();

    let stores = vec![ZustandStore {
        file_id,
        name: "useEditorStore".to_string(),
        line: 3,
    }];

    // Run twice — should not error or create duplicate members.
    create_react_concepts(&db.conn, &stores, &[]).unwrap();
    create_react_concepts(&db.conn, &stores, &[]).unwrap();

    let member_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM concept_members", [], |r| r.get(0))
        .unwrap();
    assert_eq!(member_count, 1, "OR IGNORE should prevent duplicate member");
}

#[test]
fn create_react_concepts_adds_storybook_concept() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/Button.stories.tsx', 'h2', 'tsx', 0)",
        [],
    )
    .unwrap();
    let story_file_id: i64 = conn.last_insert_rowid();

    // A symbol in the story file.
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'Default', 'ButtonStories.Default', 'variable', 5, 0)",
        [story_file_id],
    )
    .unwrap();

    let stories = vec![StoryMapping {
        story_file_id,
        component_name: "Button".to_string(),
        component_file_path: None,
    }];

    create_react_concepts(conn, &[], &stories).unwrap();

    let concept_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM concepts WHERE name = 'storybook-stories'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(concept_count, 1);
}

#[test]
fn empty_inputs_produce_no_concepts() {
    let db = Database::open_in_memory().unwrap();
    create_react_concepts(&db.conn, &[], &[]).unwrap();

    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM concepts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0, "No concepts should be created from empty inputs");
}
