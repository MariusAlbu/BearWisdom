// =============================================================================
// connectors/react_patterns.rs  —  React ecosystem pattern connector
//
// Two detection passes:
//
//   1. Zustand stores — find `const use*Store = create<...>()` in TS/TSX files.
//      Creates a "zustand-stores" concept and adds each store's symbol as a member.
//
//   2. Storybook stories — find `*.stories.tsx` / `*.stories.ts` files.
//      Extract the component name from `export default { component: Foo }` or
//      `const meta: Meta<typeof Foo>`.  Creates a "storybook-stories" concept
//      and adds each story file symbol as a member.
//
// Concept membership uses `auto_assigned = 1` to mark these as automatically
// detected rather than manually curated.
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::Connection;
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A Zustand store definition in a TypeScript file.
#[derive(Debug, Clone)]
pub struct ZustandStore {
    /// `files.id` of the containing file.
    pub file_id: i64,
    /// The variable name (e.g. `useEditorStore`).
    pub name: String,
    /// 1-based line of the `const use* = create` declaration.
    pub line: u32,
}

/// The mapping between a Storybook story file and its component.
#[derive(Debug, Clone)]
pub struct StoryMapping {
    /// `files.id` of the `.stories.tsx` file.
    pub story_file_id: i64,
    /// The component name extracted from the default export or Meta type.
    pub component_name: String,
    /// Relative path of the component file, if it can be resolved.
    pub component_file_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Find all Zustand store definitions in indexed TS/TSX files.
///
/// Files are read from disk via `project_root`.
pub fn find_zustand_stores(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<ZustandStore>> {
    let re_store = build_store_regex();

    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE language IN ('typescript', 'tsx')",
        )
        .context("Failed to prepare TS files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TS files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TS file rows")?;

    let mut stores: Vec<ZustandStore> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable TS file");
                continue;
            }
        };

        for (line_idx, line_text) in source.lines().enumerate() {
            let line_no = (line_idx + 1) as u32;
            if let Some(cap) = re_store.captures(line_text) {
                stores.push(ZustandStore {
                    file_id,
                    name: cap[1].to_string(),
                    line: line_no,
                });
            }
        }
    }

    debug!(count = stores.len(), "Zustand stores found");
    Ok(stores)
}

/// Find all Storybook story files in the index and extract component mappings.
///
/// Files are read from disk via `project_root`.
pub fn find_story_mappings(
    conn: &Connection,
    project_root: &Path,
) -> Result<Vec<StoryMapping>> {
    let re_default_export = build_default_export_regex();
    let re_meta_type = build_meta_type_regex();

    // Query only .stories.tsx / .stories.ts files.
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM files
             WHERE (path LIKE '%.stories.tsx' OR path LIKE '%.stories.ts')
               AND language IN ('typescript', 'tsx')",
        )
        .context("Failed to prepare story files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query story files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect story file rows")?;

    let mut mappings: Vec<StoryMapping> = Vec::new();

    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable story file");
                continue;
            }
        };

        let component_name = extract_component_name(&source, &re_default_export, &re_meta_type);

        let component_name = match component_name {
            Some(n) => n,
            None => {
                debug!(path = %rel_path, "Could not extract component name from story file");
                continue;
            }
        };

        // Try to find the component file path in the DB.
        let component_file_path: Option<String> = conn
            .query_row(
                "SELECT f.path FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.name = ?1 AND s.kind = 'class' OR s.kind = 'function'
                 LIMIT 1",
                [&component_name],
                |r| r.get(0),
            )
            .optional();

        mappings.push(StoryMapping {
            story_file_id: file_id,
            component_name,
            component_file_path,
        });
    }

    debug!(count = mappings.len(), "Story mappings found");
    Ok(mappings)
}

/// Create concept entries for detected stores and stories.
///
/// Ensures the "zustand-stores" and "storybook-stories" concepts exist, then
/// adds each detected item as a member with `auto_assigned = 1`.
pub fn create_react_concepts(
    conn: &Connection,
    stores: &[ZustandStore],
    stories: &[StoryMapping],
) -> Result<()> {
    if !stores.is_empty() {
        let concept_id = upsert_concept(conn, "zustand-stores", "Zustand state stores")?;

        for store in stores {
            // Find the symbol for this store by name and file_id.
            let symbol_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM symbols WHERE file_id = ?1 AND name = ?2 LIMIT 1",
                    rusqlite::params![store.file_id, store.name],
                    |r| r.get(0),
                )
                .optional();

            if let Some(sym_id) = symbol_id {
                add_concept_member(conn, concept_id, sym_id)?;
            } else {
                debug!(
                    store = %store.name,
                    "Zustand store symbol not found in index — concept member not added"
                );
            }
        }

        info!(
            stores = stores.len(),
            "React patterns: zustand-stores concept updated"
        );
    }

    if !stories.is_empty() {
        let concept_id = upsert_concept(conn, "storybook-stories", "Storybook story files")?;

        for story in stories {
            // Add any symbols in the story file as members.
            let symbol_ids: Vec<i64> = {
                let mut stmt = conn
                    .prepare("SELECT id FROM symbols WHERE file_id = ?1")
                    .context("Failed to prepare story symbol query")?;
                let rows: rusqlite::Result<Vec<i64>> =
                    stmt.query_map([story.story_file_id], |r| r.get(0))?.collect();
                rows.context("Failed to collect story symbol ids")?
            };

            for sym_id in symbol_ids {
                add_concept_member(conn, concept_id, sym_id)?;
            }
        }

        info!(
            stories = stories.len(),
            "React patterns: storybook-stories concept updated"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Regex for Zustand store exports.
///
/// Matches:
///   export const useEditorStore = create<...>
///   const useAuthStore = create(
///   export const useMyStore = create<MyState>(
fn build_store_regex() -> Regex {
    Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
        .expect("store regex is valid")
}

/// Regex for Storybook default export component: `{ component: Foo }`.
fn build_default_export_regex() -> Regex {
    Regex::new(r"component\s*:\s*(\w+)").expect("default export regex is valid")
}

/// Regex for Storybook Meta type: `Meta<typeof Foo>` or `Meta<Foo>`.
fn build_meta_type_regex() -> Regex {
    Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>").expect("meta type regex is valid")
}

/// Extract component name from story file source text.
///
/// Tries `component: Foo` first (default export object form), then
/// `Meta<typeof Foo>` (CSF3 `const meta: Meta<typeof Foo>` form).
fn extract_component_name(source: &str, re_default: &Regex, re_meta: &Regex) -> Option<String> {
    // Scan all lines — the meta or component entry can be anywhere.
    for line in source.lines() {
        if let Some(cap) = re_meta.captures(line) {
            return Some(cap[1].to_string());
        }
    }
    for line in source.lines() {
        if let Some(cap) = re_default.captures(line) {
            return Some(cap[1].to_string());
        }
    }
    None
}

/// Ensure a concept with `name` exists, returning its id.
fn upsert_concept(conn: &Connection, name: &str, description: &str) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO concepts (name, description) VALUES (?1, ?2)",
        rusqlite::params![name, description],
    )
    .context("Failed to upsert concept")?;

    let id: i64 = conn
        .query_row(
            "SELECT id FROM concepts WHERE name = ?1",
            [name],
            |r| r.get(0),
        )
        .context("Failed to fetch concept id")?;

    Ok(id)
}

/// Add a symbol to a concept with `auto_assigned = 1`.
fn add_concept_member(conn: &Connection, concept_id: i64, symbol_id: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO concept_members (concept_id, symbol_id, auto_assigned)
         VALUES (?1, ?2, 1)",
        rusqlite::params![concept_id, symbol_id],
    )
    .context("Failed to insert concept member")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Extension trait for rusqlite::Connection
// ---------------------------------------------------------------------------

trait OptionalExt<T> {
    fn optional(self) -> Option<T>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> Option<T> {
        match self {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
