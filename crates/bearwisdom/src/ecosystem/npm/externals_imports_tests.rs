// =============================================================================
// ecosystem/npm/externals_imports_tests.rs — bare re-export / side-import scan
//
// The transitive externals walker pulls cross-package specifiers out of a
// package's type-entry through `collect_bare_reexports_recursive`, which in
// turn relies on `extract_bare_reexport_specifiers` recognising every shape
// that names another package: `export { X } from 'pkg'`, `export * from
// 'pkg'`, and the bare side import `import * as ns from 'pkg'`. The last one
// is load-bearing — a member-declaring assertion lib is reached through an
// `import * as` line in the entry of the package that re-exports the matcher
// container, with no corresponding `export ... from`. These tests pin the
// recognised shapes structurally; the package names are SYNTHETIC stand-ins,
// nothing keys on a framework name.
// =============================================================================

use super::*;

#[test]
fn extracts_named_reexport_specifier() {
    let specs = extract_bare_reexport_specifiers(
        "export { Assertion, JestAssertion } from 'runner-expect';\n",
    );
    assert_eq!(specs, vec!["runner-expect".to_string()]);
}

#[test]
fn extracts_bare_side_import_specifier() {
    // `import * as ns from 'pkg'` is a side import that brings the package's
    // namespace into scope without re-exporting any name. The matcher
    // container's peer assertion lib is reached only through this shape, so
    // it must be collected as a cross-package specifier.
    let specs = extract_bare_reexport_specifiers("import * as bdd from 'bdd-assert';\n");
    assert_eq!(specs, vec!["bdd-assert".to_string()]);
}

#[test]
fn extracts_plain_named_side_import_specifier() {
    // `import { X } from 'pkg'` (no namespace) is the other side-import shape
    // the entry uses to wire peer types in. Same package portion expected.
    let specs = extract_bare_reexport_specifiers("import { ExpectStatic } from 'runner-expect';\n");
    assert_eq!(specs, vec!["runner-expect".to_string()]);
}

#[test]
fn reduces_scoped_specifier_to_package_portion() {
    // A scoped deep specifier reduces to `@scope/pkg`; the in-package path is
    // dropped. The walker walks the whole package from there.
    let specs = extract_bare_reexport_specifiers("export { Page } from '@scope/pkg/sub/path';\n");
    assert_eq!(specs, vec!["@scope/pkg".to_string()]);
}

#[test]
fn skips_relative_specifiers() {
    // Relative re-exports stay within the package and are followed by the
    // relative-chain walker, not the bare-specifier collector.
    let specs = extract_bare_reexport_specifiers(
        "export { X } from './local';\nexport * from '../sibling';\n",
    );
    assert!(specs.is_empty(), "{specs:?}");
}

#[test]
fn extracts_specifier_from_multi_line_named_clause() {
    // Type-entry files routinely break a long named clause across physical
    // lines; the `from '<spec>'` clause then sits on a continuation line. The
    // logical-line collapse must join them so the specifier is still seen.
    let src = "export {\n  Assertion,\n  JestAssertion,\n  ExpectStatic,\n} from 'runner-expect';\n";
    let specs = extract_bare_reexport_specifiers(src);
    assert_eq!(specs, vec!["runner-expect".to_string()]);
}

#[test]
fn recursive_collector_follows_relative_chain_to_bare_specifier() {
    // The entry re-exports a relative module which itself side-imports a peer
    // package. The recursive collector must walk the relative hop and surface
    // the bare specifier declared one file deeper — the exact shape that
    // reaches a matcher peer hidden behind a barrel.
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    std::fs::write(dir.join("index.d.ts"), "export * from './chunk';\n").unwrap();
    std::fs::write(
        dir.join("chunk.d.ts"),
        "import * as bdd from 'bdd-assert';\nexport { Assertion } from 'runner-expect';\n",
    )
    .unwrap();

    let mut specs = collect_bare_reexports_recursive(&dir.join("index.d.ts"));
    specs.sort();
    assert_eq!(
        specs,
        vec!["bdd-assert".to_string(), "runner-expect".to_string()],
        "recursive collector must surface bare specifiers across the relative chain"
    );
}
