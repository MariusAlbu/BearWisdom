//! An untyped package whose declarations ship in its DefinitelyTyped
//! companion binds through the companion: `import { useState } from 'react'`
//! reaches `@types/react/index.d.ts`, and a local `declare module 'react'`
//! augmentation never claims the package's entry.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn seed_node_modules() -> TempDir {
    let nm = TempDir::new().unwrap();
    let react = nm.path().join("react");
    fs::create_dir_all(&react).unwrap();
    fs::write(
        react.join("package.json"),
        r#"{"name":"react","version":"18.0.0","main":"index.js"}"#,
    )
    .unwrap();
    fs::write(
        react.join("index.js"),
        "'use strict';\nmodule.exports = require('./cjs/react.development.js');\n",
    )
    .unwrap();
    let types = nm.path().join("@types").join("react");
    fs::create_dir_all(&types).unwrap();
    fs::write(
        types.join("package.json"),
        r#"{"name":"@types/react","version":"18.0.0","types":"index.d.ts"}"#,
    )
    .unwrap();
    fs::write(
        types.join("index.d.ts"),
        "export = React;\nexport as namespace React;\ndeclare namespace React {\n    function useState<S>(initial: S): [S, (next: S) => void];\n    function useRef<T>(initial: T): { current: T };\n    interface ReactNode {}\n}\n",
    )
    .unwrap();
    nm
}

fn seed_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };
    project.add_file(
        "package.json",
        r#"{"name":"companion","version":"0.0.1","dependencies":{"react":"^18.0.0"},"devDependencies":{"@types/react":"^18.0.0"}}"#,
    );
    project.add_file(
        "src/counter.ts",
        "import { useState, useRef } from 'react';\nimport type { ReactNode } from 'react';\n\nexport function counter(): ReactNode {\n  const [n, setN] = useState(0);\n  const r = useRef(1);\n  setN(n + 1);\n  return null as unknown as ReactNode;\n}\n\ndeclare module 'react' {\n  interface ReactNode { extra?: boolean }\n}\n",
    );
    project
}

static NODE_MODULES_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn an_untyped_package_binds_through_its_types_companion() {
    let node_modules = seed_node_modules();
    let project = seed_project();
    let _serial = NODE_MODULES_ENV
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prior = std::env::var_os("BEARWISDOM_TS_NODE_MODULES");
    unsafe {
        std::env::set_var("BEARWISDOM_TS_NODE_MODULES", node_modules.path());
    }
    let mut db = TestProject::in_memory_db();
    let indexed = full_index(&mut db, project.path(), None, None, None);
    unsafe {
        match prior {
            Some(v) => std::env::set_var("BEARWISDOM_TS_NODE_MODULES", v),
            None => std::env::remove_var("BEARWISDOM_TS_NODE_MODULES"),
        }
    }
    indexed.unwrap();



    let companion_files: Vec<String> = {
        let mut stmt = db
            .prepare("SELECT path FROM files WHERE origin = 'external' AND path LIKE '%@types/react%' ORDER BY path")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        companion_files,
        vec!["ext:ts:@types/react/index.d.ts".to_string()],
        "the companion's entry is materialized for the owner's import"
    );

    let bound: Vec<(String, String, String)> = {
        let mut stmt = db
            .prepare(
                "SELECT s.name, e.kind, t.name FROM edges e
                 JOIN symbols s ON s.id = e.source_id
                 JOIN symbols t ON t.id = e.target_id
                 JOIN files f ON f.id = t.file_id
                 WHERE f.path = 'ext:ts:@types/react/index.d.ts'
                 ORDER BY s.name, e.kind, t.name",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for expected in [
        ("counter", "calls", "useState"),
        ("counter", "calls", "useRef"),
        ("counter", "type_ref", "ReactNode"),
    ] {
        let expected = (expected.0.to_string(), expected.1.to_string(), expected.2.to_string());
        assert!(bound.contains(&expected), "missing {expected:?} in {bound:?}");
    }

    let hijacked: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e JOIN symbols t ON t.id = e.target_id JOIN files f ON f.id = t.file_id
             WHERE t.name IN ('useState','useRef') AND f.path = 'src/counter.ts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hijacked, 0, "the local augmentation never becomes the package entry");
}
