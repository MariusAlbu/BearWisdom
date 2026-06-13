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
    fs::write(root.join("src/generated/db.ts"), "export class Db {}\n").unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    assert_eq!(
        extra.len(),
        1,
        "expected exactly one extra file; got {extra:?}"
    );
    assert!(
        extra[0].relative_path.ends_with("generated/db.ts"),
        "expected the gitignored file; got {}",
        extra[0].relative_path
    );
}

#[test]
fn pulls_transitive_gitignored_chain() {
    // The generated-client shape: project source imports the gitignored
    // re-export file, which imports a second gitignored file, which defines
    // the delegate. Only the first hop is directly imported from primary —
    // the rest are reachable only by following pulled files' own imports.
    //   src/app.ts                          (walked, imports './gen/client')
    //   src/gen/client.ts                    (gitignored, re-exports './internal/class')
    //   src/gen/internal/class.ts            (gitignored, imports '../models/User')
    //   src/gen/models/User.ts               (gitignored, defines the delegate)
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src/gen/internal")).unwrap();
    fs::create_dir_all(root.join("src/gen/models")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        "import { PrismaClient } from './gen/client';\n",
    )
    .unwrap();
    fs::write(
        root.join("src/gen/client.ts"),
        "export * from './internal/class';\n",
    )
    .unwrap();
    fs::write(
        root.join("src/gen/internal/class.ts"),
        "import { UserDelegate } from '../models/User';\nexport interface PrismaClient { get user(): UserDelegate; }\n",
    )
    .unwrap();
    fs::write(
        root.join("src/gen/models/User.ts"),
        "export interface UserDelegate { findMany(): User[]; }\n",
    )
    .unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    let names: Vec<&str> = extra.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(
        names.iter().any(|n| n.ends_with("gen/client.ts")),
        "first hop missing: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("internal/class.ts")),
        "second hop (transitive) missing: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("models/User.ts")),
        "third hop (transitive) missing: {names:?}"
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
    fs::write(root.join("src/generated/db.ts"), "export class Db {}\n").unwrap();

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
    fs::write(root.join("src/app.ts"), "import React from 'react';\n").unwrap();
    fs::write(
        root.join("node_modules/react/index.d.ts"),
        "export default class React {}\n",
    )
    .unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    assert!(
        extra
            .iter()
            .all(|f| !f.relative_path.contains("node_modules")),
        "must not pull from node_modules: {extra:?}"
    );
}

#[test]
fn does_not_duplicate_primary_files() {
    // The file IS in primary; we shouldn't add it again.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/app.ts"), "import { x } from './lib';\n").unwrap();
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
    assert!(
        names.iter().any(|n| n.ends_with("sub.ts")),
        "export-from missing: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("star.ts")),
        "export-star missing: {names:?}"
    );
}

#[test]
fn empty_primary_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let extra = pull_gitignored_imports(tmp.path(), &[]);
    assert!(extra.is_empty());
}

// ---------------------------------------------------------------------------
// lexically_normalize — pure-lexical `.`/`..` folding, no filesystem access.
// ---------------------------------------------------------------------------

#[test]
fn normalize_folds_parent_segment() {
    assert_eq!(lexically_normalize(Path::new("a/b/../c")), PathBuf::from("a/c"));
}

#[test]
fn normalize_folds_repeated_parents() {
    assert_eq!(
        lexically_normalize(Path::new("a/b/c/../../d")),
        PathBuf::from("a/d")
    );
}

#[test]
fn normalize_drops_current_dir_segment() {
    assert_eq!(lexically_normalize(Path::new("a/./b")), PathBuf::from("a/b"));
}

#[test]
fn normalize_noop_on_already_canonical() {
    assert_eq!(
        lexically_normalize(Path::new("a/b/c.ts")),
        PathBuf::from("a/b/c.ts")
    );
}

#[test]
fn normalize_collapses_repeated_and_trailing_separators() {
    // Component iteration drops empty segments from `//` and a trailing `/`.
    assert_eq!(
        lexically_normalize(Path::new("a//b/")),
        PathBuf::from("a/b")
    );
}

#[test]
fn normalize_keeps_leading_parent_escaping_anchor() {
    // A `..` with nothing poppable above it is retained — folding past the
    // root is never attempted.
    assert_eq!(
        lexically_normalize(Path::new("../sibling/x")),
        PathBuf::from("../sibling/x")
    );
    assert_eq!(
        lexically_normalize(Path::new("a/../../x")),
        PathBuf::from("../x")
    );
}

#[test]
fn normalize_preserves_forward_slash_relative_root_anchor() {
    // A rooted path keeps its root and never pops the anchor.
    let normed = lexically_normalize(Path::new("/a/b/../c"));
    assert_eq!(normed, PathBuf::from("/a/c"));
}

// ---------------------------------------------------------------------------
// Production-point integration: a relative `..` import yields a canonical,
// project-root-relative stored path — never a `/../`-bearing duplicate row.
// ---------------------------------------------------------------------------

#[test]
fn relative_parent_import_stores_canonical_path() {
    // Layout mirrors the calcom monorepo shape that produced duplicate rows:
    //   a/facade/y.ts       (walked, imports '../service/x')
    //   a/service/x.ts      (gitignored, NOT in primary)
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("a/facade")).unwrap();
    fs::create_dir_all(root.join("a/service")).unwrap();
    fs::write(
        root.join("a/facade/y.ts"),
        "import { X } from '../service/x';\n",
    )
    .unwrap();
    fs::write(root.join("a/service/x.ts"), "export class X {}\n").unwrap();

    let primary = vec![make_walked(root, "a/facade/y.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    assert_eq!(extra.len(), 1, "expected one extra file; got {extra:?}");
    assert_eq!(
        extra[0].relative_path, "a/service/x.ts",
        "stored path must be canonical, not '/../'-bearing: {}",
        extra[0].relative_path
    );
    assert!(
        !extra[0].relative_path.contains("/../"),
        "stored path leaked a '/../' segment: {}",
        extra[0].relative_path
    );
    assert!(
        !extra[0].absolute_path.to_string_lossy().contains("/../"),
        "absolute path leaked a '/../' segment: {}",
        extra[0].absolute_path.display()
    );
}

#[test]
fn parent_import_target_already_in_primary_is_not_duplicated() {
    // The `..`-resolved target is already in the primary walk under its
    // canonical path. Folding must let the dedup recognize it so no second
    // (non-canonical) row is added.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("a/facade")).unwrap();
    fs::create_dir_all(root.join("a/service")).unwrap();
    fs::write(
        root.join("a/facade/y.ts"),
        "import { X } from '../service/x';\n",
    )
    .unwrap();
    fs::write(root.join("a/service/x.ts"), "export class X {}\n").unwrap();

    let primary = vec![
        make_walked(root, "a/facade/y.ts", "typescript"),
        make_walked(root, "a/service/x.ts", "typescript"),
    ];
    let extra = pull_gitignored_imports(root, &primary);

    assert!(
        extra.is_empty(),
        "canonical twin is in primary; no duplicate expected, got {extra:?}"
    );
}

#[test]
fn bare_and_root_relative_specifiers_unaffected_by_folding() {
    // Bare specifiers still route to the externals walker (skipped here), and
    // a project-root-relative import without `..` resolves to its canonical
    // path exactly as before.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("src/generated")).unwrap();
    fs::write(
        root.join("src/app.ts"),
        "import 'react';\nimport { Db } from 'src/generated/db';\n",
    )
    .unwrap();
    fs::write(root.join("src/generated/db.ts"), "export class Db {}\n").unwrap();

    let primary = vec![make_walked(root, "src/app.ts", "typescript")];
    let extra = pull_gitignored_imports(root, &primary);

    assert_eq!(extra.len(), 1, "expected only the project-relative file; got {extra:?}");
    assert_eq!(extra[0].relative_path, "src/generated/db.ts");
    assert!(extra.iter().all(|f| !f.relative_path.contains("node_modules")));
}
