//! Integration tests for L2 — per-file manifest association.
//!
//! L2 is mostly a verification pass on top of M1+M2: the invariant it
//! formalizes is that every symbol extracted from a file (host OR
//! embedded sub-extraction) uses that file's `package_id` for manifest
//! classification. Since all symbols land in the same `ParsedFile` and
//! the resolver reads `pf.package_id` for every ref, inheritance is
//! structural — these tests lock it in.

use std::fs;
use std::path::Path;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

#[test]
fn vue_embedded_ts_inherits_host_file_package_id() {
    // Layout: two npm workspace packages. apps/web has a Vue SFC with
    // embedded TS; apps/server is unrelated. The Vue file's embedded TS
    // symbols must sit on files.package_id = apps/web and the refs they
    // produce must classify against apps/web's manifest.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "package.json",
        r#"{"name":"ws","private":true,"workspaces":["apps/web","apps/server"]}"#,
    );
    write_file(
        root,
        "apps/web/package.json",
        r#"{"name":"web","dependencies":{"vue":"3"}}"#,
    );
    // Vue SFC with an embedded TS <script> block importing vue plus a
    // top-level TS declaration (the anonymous defineComponent export
    // doesn't yield a named TS symbol; `helper` does).
    write_file(
        root,
        "apps/web/src/Button.vue",
        r#"<script lang="ts">
import { defineComponent } from 'vue';

const helper = () => 42;

export default defineComponent({
    name: 'Button',
    setup() { return { helper }; },
});
</script>
<template><button></button></template>
"#,
    );
    write_file(
        root,
        "apps/server/package.json",
        r#"{"name":"server","dependencies":{"express":"4"}}"#,
    );
    write_file(
        root,
        "apps/server/src/app.ts",
        r#"export const x = 1;"#,
    );

    let mut db = TestProject::in_memory_db();
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");
    full_index(&mut db, root, None, None, None).expect("index failed");

    // Identify packages.
    let web_id: i64 = db
        .query_row("SELECT id FROM packages WHERE name = 'web'", [], |r| r.get(0))
        .expect("web package missing");

    // The Vue file itself should have package_id = web.
    let file_pkg: Option<i64> = db
        .query_row(
            "SELECT package_id FROM files WHERE path LIKE '%Button.vue'",
            [],
            |r| r.get(0),
        )
        .expect("Button.vue row missing");
    assert_eq!(
        file_pkg,
        Some(web_id),
        "Vue host file should belong to the web package"
    );

    // At least one embedded TS symbol should exist in the Vue file.
    let embedded_ts_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE '%Button.vue'
               AND s.origin_language = 'typescript'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        embedded_ts_count > 0,
        "expected TS symbols from Vue <script lang='ts'> block, got {embedded_ts_count}"
    );

    // Any external_ref from an embedded TS symbol must carry package_id = web
    // (the host file's package), proving L2's inheritance invariant.
    let mismatched: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM external_refs er
             JOIN symbols s ON s.id = er.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.path LIKE '%Button.vue'
               AND s.origin_language = 'typescript'
               AND (er.package_id IS NULL OR er.package_id != ?1)",
            rusqlite::params![web_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        mismatched, 0,
        "embedded TS refs must inherit host file's package_id = web"
    );
}

#[test]
fn root_script_falls_back_to_union_manifest() {
    // A shared script at the workspace root has no package_id (it sits
    // above every package). It still needs to classify external imports
    // as external — M2's `manifests_for(None)` returns the union so that
    // works. This test proves the end-to-end flow: a .ts at root imports
    // a dep declared only in a child package and still gets classified
    // against the union.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "package.json",
        r#"{"name":"ws","private":true,"workspaces":["apps/web"]}"#,
    );
    write_file(
        root,
        "apps/web/package.json",
        r#"{"name":"web","dependencies":{"lodash":"4"}}"#,
    );
    write_file(
        root,
        "apps/web/src/index.ts",
        r#"export const x = 1;"#,
    );
    // Root-level shared script — no package_id.
    write_file(
        root,
        "scripts/build.ts",
        r#"import { debounce } from 'lodash';
export const deb = debounce;
"#,
    );

    let mut db = TestProject::in_memory_db();
    std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");
    full_index(&mut db, root, None, None, None).expect("index failed");

    // Root script must have NULL package_id (it sits outside apps/web).
    let root_pkg: Option<i64> = db
        .query_row(
            "SELECT package_id FROM files WHERE path LIKE '%scripts/build.ts'
                OR path LIKE '%scripts\\build.ts'",
            [],
            |r| r.get(0),
        )
        .expect("build.ts row missing");
    assert_eq!(root_pkg, None, "root-level script should have no package_id");

    // `lodash` is declared in apps/web only. Since root script's
    // package_id is None, manifests_for(None) returns the union which
    // INCLUDES lodash — so the import MUST classify as external (not
    // unresolved).
    let lodash_external: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM external_refs er
             JOIN symbols s ON s.id = er.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE (f.path LIKE '%scripts/build.ts' OR f.path LIKE '%scripts\\build.ts')
               AND er.namespace = 'lodash'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        lodash_external > 0,
        "root script's `import from lodash` must classify as external via union fallback"
    );
}
