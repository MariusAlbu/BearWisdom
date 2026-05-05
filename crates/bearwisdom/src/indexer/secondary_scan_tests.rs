use super::*;
use std::fs;
use tempfile::TempDir;

fn make_walked(root: &Path, rel: &str, lang: &'static str) -> WalkedFile {
    WalkedFile {
        relative_path: rel.to_string(),
        absolute_path: root.join(rel),
        language: lang,
    }
}

#[test]
fn pulls_gitignored_relative_import() {
    // Project layout:
    //   src/app.ts          (walked, imports './generated/db')
    //   src/generated/db.ts (gitignored, NOT in primary)
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src/generated")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        "import { Db } from './generated/db';\nexport const x = 1;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/generated/db.ts"),
        "export class Db {}\n",
    )
    .unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    assert_eq!(extra.len(), 1, "expected exactly one extra file; got {extra:?}");
    assert!(
        extra[0].relative_path.ends_with("generated/db.ts"),
        "expected the gitignored file; got {}",
        extra[0].relative_path
    );
}

#[test]
fn pulls_project_relative_import() {
    // `import { Db } from 'src/generated/db'` (no leading `./`).
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src/generated")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        "import { Db } from 'src/generated/db';\nexport const x = 1;\n",
    )
    .unwrap();
    fs::write(
        root.join("src/generated/db.ts"),
        "export class Db {}\n",
    )
    .unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    assert_eq!(extra.len(), 1);
    assert!(extra[0].relative_path.contains("generated/db"));
}

#[test]
fn skips_node_modules_imports() {
    // A standard `import from 'react'` must NOT cause us to walk node_modules.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("node_modules/react")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        "import React from 'react';\n",
    )
    .unwrap();
    fs::write(
        root.join("node_modules/react/index.d.ts"),
        "export default class React {}\n",
    )
    .unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    assert!(
        extra.iter().all(|f| !f.relative_path.contains("node_modules")),
        "must not pull from node_modules: {extra:?}"
    );
}

#[test]
fn does_not_duplicate_primary_files() {
    // The file IS in primary; we shouldn't add it again.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        "import { x } from './lib';\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.ts"), "export const x = 1;\n").unwrap();

    let primary = vec![
        make_walked(root, "src/app.ts", "typescript"),
        make_walked(root, "src/lib.ts", "typescript"),
    ];
    let extra = pull_gitignored_imports(root, &primary);

    assert!(extra.is_empty(), "lib.ts already in primary; got {extra:?}");
}

#[test]
fn resolves_index_files() {
    // `import './foo'` where `foo` is a directory containing `index.ts`.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src/generated/prisma")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        "import { Db } from './generated/prisma';\n",
    )
    .unwrap();
    fs::write(
        root.join("src/generated/prisma/index.ts"),
        "export class Db {}\n",
    )
    .unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    assert_eq!(extra.len(), 1, "expected index.ts; got {extra:?}");
    assert!(extra[0].relative_path.ends_with("prisma/index.ts"));
}

#[test]
fn handles_dynamic_import_and_require() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src/generated")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        r#"
const a = require('./generated/cjs');
const b = await import('./generated/dyn');
"#,
    )
    .unwrap();
    fs::write(root.join("src/generated/cjs.js"), "module.exports = {};\n").unwrap();
    fs::write(root.join("src/generated/dyn.ts"), "export const dyn = 1;\n").unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    let names: Vec<&str> = extra.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(
        names.iter().any(|n| n.ends_with("cjs.js")),
        "require() target missing: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("dyn.ts")),
        "dynamic import target missing: {names:?}"
    );
}

#[test]
fn handles_export_from() {
    // `export { x } from './sub'` and `export * from './sub'`.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        r#"
export { x } from './sub';
export * from './star';
"#,
    )
    .unwrap();
    fs::write(root.join("src/sub.ts"), "export const x = 1;\n").unwrap();
    fs::write(root.join("src/star.ts"), "export const y = 2;\n").unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    let names: Vec<&str> = extra.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(names.iter().any(|n| n.ends_with("sub.ts")), "export-from missing: {names:?}");
    assert!(names.iter().any(|n| n.ends_with("star.ts")), "export-star missing: {names:?}");
}

#[test]
fn empty_primary_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let extra = pull_gitignored_imports(tmp.path(), &[]);
    assert!(extra.is_empty());
}
