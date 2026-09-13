//! Subpath demand is a property of the module specifier, not of the workspace
//! package that happened to find a copy of the module first.
//!
//! The monorepo below mirrors the real shape: the package that imports the deep
//! subpaths reaches only an UNBUILT copy of `pkg`, while the copy carrying the
//! built `dist/` output belongs to a package that imports nothing but the bare
//! specifier. Only a module-wide union of demand lets the built copy be probed
//! for the subpaths its own package never asked for.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// The consumer half — identical in both layouts. `a-consumer` sorts before
/// `b-host`, so it is the package that claims the hoisted root copy.
fn seed_consumer(project: &TestProject) {
    project.add_file(
        "package.json",
        r#"{"name":"ws","private":true,"workspaces":["apps/*"],"devDependencies":{"pkg":"*"}}"#,
    );
    project.add_file(
        "node_modules/pkg/package.json",
        r#"{"name":"pkg","types":"index.d.ts"}"#,
    );
    project.add_file("node_modules/pkg/index.d.ts", "export {};\n");
    // The unbuilt copy forwards to a `dist/` that was never built.
    project.add_file(
        "node_modules/pkg/link.d.ts",
        "export * from './dist/link';\n",
    );
    project.add_file(
        "apps/a-consumer/package.json",
        r#"{"name":"a-consumer","dependencies":{"pkg":"*"}}"#,
    );
    project.add_file(
        "apps/a-consumer/src/app.ts",
        "import { Link } from 'pkg/link';\nimport { Legacy } from 'pkg/legacy/image';\n\nexport const use = (): string => Link.href + Legacy.src;\n",
    );
    project.add_file(
        "apps/b-host/package.json",
        r#"{"name":"b-host","dependencies":{"pkg":"*"}}"#,
    );
    project.add_file(
        "apps/b-host/src/host.ts",
        "import def from 'pkg';\nexport const d = def;\n",
    );
}

/// `b-host`'s own copy is the BUILT one: it carries `dist/link.d.ts` and the
/// nested `legacy/image.d.ts`, neither of which `b-host` itself imports.
fn seed_built_host_copy(project: &TestProject) {
    project.add_file(
        "apps/b-host/node_modules/pkg/package.json",
        r#"{"name":"pkg","types":"index.d.ts"}"#,
    );
    project.add_file(
        "apps/b-host/node_modules/pkg/index.d.ts",
        "declare const def: {};\nexport default def;\n",
    );
    project.add_file(
        "apps/b-host/node_modules/pkg/link.d.ts",
        "export * from './dist/link';\n",
    );
    project.add_file(
        "apps/b-host/node_modules/pkg/dist/link.d.ts",
        "export declare const Link: { href: string };\n",
    );
    project.add_file(
        "apps/b-host/node_modules/pkg/legacy/image.d.ts",
        "export declare const Legacy: { src: string };\n",
    );
}

fn index(project: &TestProject) -> bearwisdom::Database {
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None, None).unwrap();
    db
}

/// `(ref name, external file the edge lands on)` for every edge out of the
/// consumer's source file.
fn consumer_external_targets(db: &bearwisdom::Database) -> Vec<(String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT t.name, f.path FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files sf ON sf.id = s.file_id
             JOIN symbols t ON t.id = e.target_id
             JOIN files f ON f.id = t.file_id
             WHERE sf.path = 'apps/a-consumer/src/app.ts' AND f.path LIKE 'ext:ts:%'
             ORDER BY t.name, f.path",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// Undrained refs still filed as "the module was declared but nothing supplied
/// it" for the two demanded subpaths — the bucket the union must empty.
fn unsupplied_subpath_refs(db: &bearwisdom::Database) -> Vec<(String, String)> {
    let mut stmt = db
        .prepare(
            "SELECT module, target_name FROM unresolved_refs
             WHERE drained = 0
               AND cause_kind = 'unbound_import_declared_unsupplied'
               AND module IN ('pkg/link', 'pkg/legacy/image')
             ORDER BY module, target_name",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn another_packages_demand_probes_the_copy_that_has_the_files() {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    seed_consumer(&project);
    seed_built_host_copy(&project);
    let db = index(&project);

    let targets = consumer_external_targets(&db);
    assert!(
        targets.contains(&("Link".to_string(), "ext:ts:pkg/dist/link.d.ts".to_string())),
        "`pkg/link` must bind through the built copy's re-export hop: {targets:?}"
    );
    assert!(
        targets.contains(&(
            "Legacy".to_string(),
            "ext:ts:pkg/legacy/image.d.ts".to_string()
        )),
        "a multi-segment demanded subpath must be probed: {targets:?}"
    );

    let unsupplied = unsupplied_subpath_refs(&db);
    assert!(
        unsupplied.is_empty(),
        "both subpath modules are supplied: {unsupplied:?}"
    );
}

#[test]
fn demand_does_not_invent_supply_when_no_copy_has_the_files() {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    seed_consumer(&project);
    let db = index(&project);

    let targets = consumer_external_targets(&db);
    assert!(
        !targets
            .iter()
            .any(|(name, _)| name == "Link" || name == "Legacy"),
        "no copy declares these symbols; nothing may bind: {targets:?}"
    );
}
