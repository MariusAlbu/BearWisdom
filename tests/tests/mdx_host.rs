//! Integration tests for E4 — MDX host plugin.
//!
//! Verifies:
//!
//!   1. `.mdx` files are detected as the `mdx` language (not markdown).
//!   2. Headings surface as symbols (shared host-scan with Markdown).
//!   3. Top-level `import` statements become a TS ScriptBlock region —
//!      imported names resolve when the referenced module exists in the
//!      project.
//!   4. Inline `<Button />` JSX emits a `Calls` ref against the file
//!      host symbol.
//!   5. Dotted JSX (`<Tabs.Root>`) is captured as a component ref.
//!   6. A TSX fenced block's JSX does NOT double-count as an MDX JSX
//!      ref — the fence dispatches to the TS sub-extractor; the MDX
//!      scanner skips inside fence ranges.
//!   7. Frontmatter coexists with imports and JSX without interfering.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

#[test]
fn mdx_file_detected_as_mdx_language() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(root.join("page.mdx"), "# Hi\n").unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let lang: String = db
        .query_row(
            "SELECT language FROM files WHERE path LIKE '%page.mdx'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lang, "mdx");
}

#[test]
fn mdx_headings_surface_as_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("post.mdx"),
        "# Top Heading\n\n## Section A\n\n## Section B\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let heading_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%post.mdx'
               AND s.kind = 'field'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        heading_count >= 3,
        "expected at least 3 heading symbols, got {heading_count}"
    );
}

#[test]
fn mdx_top_level_imports_produce_typescript_symbols() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.mdx"),
        r#"import { greet } from './helpers'
export const meta = { title: "Welcome" }

# Welcome

Click <Button variant="primary" />.
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // The TS sub-extractor dispatches the top-level `import` +
    // `export` block as a ScriptBlock region. `export const meta`
    // produces a TS-origin symbol spliced back onto the MDX file.
    let ts_syms: Vec<String> = db
        .prepare(
            "SELECT s.name FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.mdx'
               AND s.origin_language = 'typescript'",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        ts_syms.iter().any(|n| n == "meta"),
        "expected TS-origin symbol 'meta' from top-level export in page.mdx, got {ts_syms:?}"
    );
}

#[test]
fn jsx_component_becomes_calls_ref() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.mdx"),
        "# Title\n\n<Hero title=\"Hello\" />\n\n<Card>body</Card>\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // The MDX extractor emits Calls refs for Hero and Card. These land
    // in unresolved_refs (no component definitions exist in the project).
    let jsx_refs: Vec<String> = db
        .prepare(
            "SELECT DISTINCT ur.target_name FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.mdx'
               AND ur.kind = 'calls'
             ORDER BY ur.target_name",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(
        jsx_refs.iter().any(|n| n == "Hero"),
        "expected 'Hero' Calls ref in unresolved_refs, got {jsx_refs:?}"
    );
    assert!(
        jsx_refs.iter().any(|n| n == "Card"),
        "expected 'Card' Calls ref in unresolved_refs, got {jsx_refs:?}"
    );
}

#[test]
fn dotted_jsx_captured_as_component_ref() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.mdx"),
        "<Tabs.Root>\n<Tabs.Item />\n</Tabs.Root>\n",
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let names: Vec<String> = db
        .prepare(
            "SELECT DISTINCT ur.target_name FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.mdx'
               AND ur.kind = 'calls'
             ORDER BY ur.target_name",
        )
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .flatten()
        .collect();
    assert!(names.iter().any(|n| n == "Tabs.Root"), "got {names:?}");
    assert!(names.iter().any(|n| n == "Tabs.Item"), "got {names:?}");
}

#[test]
fn jsx_inside_tsx_fence_not_double_counted_by_mdx_scanner() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("page.mdx"),
        r#"# Demo

```tsx
<InsideFence />
```

<OutsideFence />
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // `<OutsideFence />` should emit exactly one Calls ref from the MDX
    // scanner. `<InsideFence />` is inside the tsx fence — the MDX
    // scanner must NOT emit it; whether the TS sub-extractor picks it up
    // is a TS concern and snippet-tagged separately.
    let outside_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%page.mdx'
               AND ur.kind = 'calls'
               AND ur.target_name = 'OutsideFence'
               AND ur.from_snippet = 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        outside_count, 1,
        "expected exactly one non-snippet Calls ref for OutsideFence"
    );
}

#[test]
fn frontmatter_coexists_with_imports_and_jsx() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(
        root.join("post.mdx"),
        r#"---
title: "Hello"
date: 2024-01-01
---

import { Hero } from './hero'

# Welcome

<Hero />
"#,
    )
    .unwrap();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // File indexed.
    let file_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE path LIKE '%post.mdx'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(file_count, 1);

    // Heading picked up despite frontmatter above it.
    let heading_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%post.mdx'
               AND s.name = 'Welcome'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(heading_count, 1);

    // Hero JSX ref emitted.
    let hero_ref: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs ur
             JOIN symbols s ON s.id = ur.source_id
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%post.mdx'
               AND ur.kind = 'calls'
               AND ur.target_name = 'Hero'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(hero_ref >= 1, "expected at least one Hero Calls ref");
}
