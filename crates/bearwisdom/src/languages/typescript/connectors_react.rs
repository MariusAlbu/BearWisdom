// =============================================================================
// languages/typescript/connectors_react.rs — React ecosystem post-index hooks
//
// `run_react_patterns` in the connectors facade orchestrates two scans over the
// indexed TS/TSX files and writes the results into `concepts` / `concept_members`:
//
//   - Zustand `create()` calls under a `use*` const become the
//     `zustand-stores` concept.
//   - `*.stories.{ts,tsx}` files (Storybook 7+ Meta-typed) become the
//     `storybook-stories` concept, linked to every symbol in the story file.
// =============================================================================

use anyhow::{Context, Result};
use tracing::debug;

use super::connectors::OptionalExt;

// ---------------------------------------------------------------------------
// React patterns helpers (inlined from connectors/react_patterns.rs)
// ---------------------------------------------------------------------------

/// A Zustand store definition in a TypeScript file.
#[derive(Debug, Clone)]
pub struct ZustandStore {
    pub file_id: i64,
    pub name: String,
    pub line: u32,
}

/// The mapping between a Storybook story file and its component.
#[derive(Debug, Clone)]
pub struct StoryMapping {
    pub story_file_id: i64,
    pub component_name: String,
    pub component_file_path: Option<String>,
}

pub(super) fn react_find_zustand_stores(
    conn: &rusqlite::Connection,
    project_root: &std::path::Path,
) -> anyhow::Result<Vec<ZustandStore>> {
    let re_store = regex::Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
        .expect("store regex is valid");

    let mut stmt = conn
        .prepare("SELECT id, path FROM files WHERE language IN ('typescript', 'tsx')")
        .context("Failed to prepare TS files query")?;

    let files: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .context("Failed to query TS files")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect TS file rows")?;

    let mut stores = Vec::new();
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
            if let Some(cap) = re_store.captures(line_text) {
                stores.push(ZustandStore {
                    file_id,
                    name: cap[1].to_string(),
                    line: (line_idx + 1) as u32,
                });
            }
        }
    }
    debug!(count = stores.len(), "Zustand stores found");
    Ok(stores)
}

pub(super) fn react_find_story_mappings(
    conn: &rusqlite::Connection,
    project_root: &std::path::Path,
) -> anyhow::Result<Vec<StoryMapping>> {
    let re_default_export = regex::Regex::new(r"component\s*:\s*(\w+)")
        .expect("default export regex is valid");
    let re_meta_type = regex::Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>")
        .expect("meta type regex is valid");

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

    let mut mappings = Vec::new();
    for (file_id, rel_path) in files {
        let abs_path = project_root.join(&rel_path);
        let source = match std::fs::read_to_string(&abs_path) {
            Ok(s) => s,
            Err(e) => {
                debug!(path = %abs_path.display(), err = %e, "Skipping unreadable story file");
                continue;
            }
        };

        let component_name = react_extract_component_name(&source, &re_default_export, &re_meta_type);
        let component_name = match component_name {
            Some(n) => n,
            None => {
                debug!(path = %rel_path, "Could not extract component name from story file");
                continue;
            }
        };

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

fn react_extract_component_name(
    source: &str,
    re_default: &regex::Regex,
    re_meta: &regex::Regex,
) -> Option<String> {
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

pub(super) fn react_create_concepts(
    conn: &rusqlite::Connection,
    stores: &[ZustandStore],
    stories: &[StoryMapping],
) -> anyhow::Result<()> {
    if !stores.is_empty() {
        let concept_id = react_upsert_concept(conn, "zustand-stores", "Zustand state stores")?;
        for store in stores {
            let symbol_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM symbols WHERE file_id = ?1 AND name = ?2 LIMIT 1",
                    rusqlite::params![store.file_id, store.name],
                    |r| r.get(0),
                )
                .optional();
            if let Some(sym_id) = symbol_id {
                react_add_concept_member(conn, concept_id, sym_id)?;
            } else {
                debug!(store = %store.name, "Zustand store symbol not found in index — concept member not added");
            }
        }
        tracing::info!(stores = stores.len(), "React patterns: zustand-stores concept updated");
    }

    if !stories.is_empty() {
        let concept_id = react_upsert_concept(conn, "storybook-stories", "Storybook story files")?;
        for story in stories {
            let symbol_ids: Vec<i64> = {
                let mut stmt = conn
                    .prepare("SELECT id FROM symbols WHERE file_id = ?1")
                    .context("Failed to prepare story symbol query")?;
                let rows: rusqlite::Result<Vec<i64>> =
                    stmt.query_map([story.story_file_id], |r| r.get(0))?.collect();
                rows.context("Failed to collect story symbol ids")?
            };
            for sym_id in symbol_ids {
                react_add_concept_member(conn, concept_id, sym_id)?;
            }
        }
        tracing::info!(stories = stories.len(), "React patterns: storybook-stories concept updated");
    }

    Ok(())
}

fn react_upsert_concept(conn: &rusqlite::Connection, name: &str, description: &str) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO concepts (name, description) VALUES (?1, ?2)",
        rusqlite::params![name, description],
    ).context("Failed to upsert concept")?;
    let id: i64 = conn
        .query_row("SELECT id FROM concepts WHERE name = ?1", [name], |r| r.get(0))
        .context("Failed to fetch concept id")?;
    Ok(id)
}

fn react_add_concept_member(conn: &rusqlite::Connection, concept_id: i64, symbol_id: i64) -> anyhow::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO concept_members (concept_id, symbol_id, auto_assigned)
         VALUES (?1, ?2, 1)",
        rusqlite::params![concept_id, symbol_id],
    ).context("Failed to insert concept member")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn store_regex_matches_export_const() {
        let re = regex::Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
            .unwrap();
        let line = "export const useEditorStore = create<EditorState>((set) => ({";
        assert!(re.is_match(line));
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "useEditorStore");
    }

    #[test]
    fn store_regex_matches_const_without_export() {
        let re = regex::Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
            .unwrap();
        let line = "const useAuthStore = create(initializer)";
        let cap = re.captures(line).unwrap();
        assert_eq!(&cap[1], "useAuthStore");
    }

    #[test]
    fn store_regex_does_not_match_non_use_prefix() {
        let re = regex::Regex::new(r"(?:export\s+)?const\s+(use\w+)\s*=\s*create\s*[<(]")
            .unwrap();
        assert!(!re.is_match("const myState = create<State>()"));
    }

    #[test]
    fn extract_component_name_from_meta_type() {
        let re_default = regex::Regex::new(r"component\s*:\s*(\w+)").unwrap();
        let re_meta = regex::Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>").unwrap();
        let source = "const meta: Meta<typeof Button> = { title: 'Button' };\nexport default meta;";
        let name = react_extract_component_name(source, &re_default, &re_meta);
        assert_eq!(name, Some("Button".to_string()));
    }

    #[test]
    fn extract_component_name_from_default_export() {
        let re_default = regex::Regex::new(r"component\s*:\s*(\w+)").unwrap();
        let re_meta = regex::Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>").unwrap();
        let source = "export default { component: FileTree, title: 'FileTree' };";
        let name = react_extract_component_name(source, &re_default, &re_meta);
        assert_eq!(name, Some("FileTree".to_string()));
    }

    #[test]
    fn extract_component_name_meta_takes_priority() {
        let re_default = regex::Regex::new(r"component\s*:\s*(\w+)").unwrap();
        let re_meta = regex::Regex::new(r"Meta\s*<\s*(?:typeof\s+)?(\w+)\s*>").unwrap();
        let source = "const meta: Meta<typeof Button> = { component: OtherThing };";
        let name = react_extract_component_name(source, &re_default, &re_meta);
        assert_eq!(name, Some("Button".to_string()));
    }

    #[test]
    fn react_create_concepts_adds_zustand_concept() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/stores/editorStore.ts', 'h1', 'typescript', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'useEditorStore', 'useEditorStore', 'variable', 3, 0)",
            [file_id],
        ).unwrap();

        let stores = vec![ZustandStore { file_id, name: "useEditorStore".to_string(), line: 3 }];
        react_create_concepts(conn, &stores, &[]).unwrap();

        let concept_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM concepts WHERE name = 'zustand-stores'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(concept_count, 1);
    }

    #[test]
    fn react_create_concepts_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let conn = db.conn();

        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/stores/editorStore.ts', 'h1', 'typescript', 0)",
            [],
        ).unwrap();
        let file_id: i64 = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'useEditorStore', 'useEditorStore', 'variable', 3, 0)",
            [file_id],
        ).unwrap();

        let stores = vec![ZustandStore { file_id, name: "useEditorStore".to_string(), line: 3 }];
        react_create_concepts(conn, &stores, &[]).unwrap();
        react_create_concepts(conn, &stores, &[]).unwrap();

        let member_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM concept_members", [], |r| r.get(0))
            .unwrap();
        assert_eq!(member_count, 1);
    }

    #[test]
    fn empty_inputs_produce_no_concepts() {
        let db = Database::open_in_memory().unwrap();
        react_create_concepts(db.conn(), &[], &[]).unwrap();
        let count: i64 = db.conn()
            .query_row("SELECT COUNT(*) FROM concepts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
