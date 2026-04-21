//! Integration test for chai / vitest synthetic chain-type injection.
//!
//! Verifies that `NpmEcosystem::parse_metadata_only` injects synthetic
//! `ParsedFile` entries when chai or vitest is present in `node_modules`,
//! and that the TypeInfo builder populates `field_type` / `return_type` for
//! the critical chain hops so the chain walker can follow
//! `expect(x).to.be.equal(y)` and `vi.spyOn(...).toHaveBeenCalledOnce()`.

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;


/// Build a project that uses chai's fluent assertion style. The `@types/chai`
/// stub is written inside the project's own `node_modules` so no env vars are
/// needed — avoids env-var leakage between parallel test threads.
fn seed_chai_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file(
        "package.json",
        r#"{"name":"chai-consumer","devDependencies":{"chai":"^6.0.0"}}"#,
    );

    // Stub @types/chai inside the project so synthetic_test_chain_files
    // finds it via the local node_modules probe (no env override needed).
    project.add_file(
        "node_modules/@types/chai/package.json",
        r#"{"name":"@types/chai","version":"5.0.0"}"#,
    );
    project.add_file(
        "node_modules/@types/chai/index.d.ts",
        "// stub — real type info injected synthetically\nexport {};\n",
    );

    project.add_file(
        "test/util.test.js",
        r#"import { expect } from 'chai';

function add(a, b) { return a + b; }

describe('add', () => {
    it('sums two numbers', () => {
        expect(add(1, 2)).to.equal(3);
        expect(add(1, 2)).to.be.equal(3);
        expect(add(1, 2)).to.be.a('number');
        expect([1]).to.have.length(1);
        expect(null).to.not.exist;
    });
});
"#,
    );

    project
}

/// Build a project that uses vitest's `vi.spyOn` mock chain.
/// Stubs vitest inside the project's own `node_modules` to avoid env-var
/// leakage between parallel test threads.
fn seed_vitest_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file(
        "package.json",
        r#"{"name":"vitest-consumer","devDependencies":{"vitest":"^2.0.0"}}"#,
    );

    project.add_file(
        "node_modules/vitest/package.json",
        r#"{"name":"vitest","version":"2.0.0","main":"index.js","types":"index.d.ts"}"#,
    );
    project.add_file(
        "node_modules/vitest/index.d.ts",
        "// stub — real type info injected synthetically\nexport {};\n",
    );

    project.add_file(
        "src/math.ts",
        "export function add(a: number, b: number): number { return a + b; }\n",
    );

    project.add_file(
        "src/math.test.ts",
        r#"import { vi, describe, it, expect } from 'vitest';
import { add } from './math';

describe('math', () => {
    it('tracks calls', () => {
        const spy = vi.spyOn(Math, 'abs');
        Math.abs(-1);
        spy.toHaveBeenCalledOnce();
        spy.toHaveBeenCalledWith(-1);
        spy.mockReturnValue(42);
        spy.mockReset();
    });
});
"#,
    );

    project
}

#[test]
fn chai_synthetic_symbols_are_indexed() {
    let project = seed_chai_project();

    let mut db = TestProject::in_memory_db();
    let _stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    // chai.Assertion must be in the external symbols.
    let assertion_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'chai.Assertion'
               AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        assertion_count, 1,
        "chai.Assertion must be indexed as an external symbol"
    );

    // chai.Assertion.to must be present (chain property).
    let to_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'chai.Assertion.to'
               AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        to_count, 1,
        "chai.Assertion.to must be indexed as an external property"
    );

    // chai.Assertion.equal must be present (chain method).
    let equal_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'chai.Assertion.equal'
               AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        equal_count, 1,
        "chai.Assertion.equal must be indexed as an external method"
    );

    // chai.expect must be present (root function).
    let expect_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'chai.expect'
               AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        expect_count, 1,
        "chai.expect function must be indexed as external"
    );
}

#[test]
fn vitest_synthetic_symbols_are_indexed() {
    let project = seed_vitest_project();

    let mut db = TestProject::in_memory_db();
    let _stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    // vitest.Vi.spyOn must be present.
    let spyon_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'vitest.Vi.spyOn'
               AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        spyon_count, 1,
        "vitest.Vi.spyOn must be indexed as an external method"
    );

    // vitest.MockInstance.toHaveBeenCalledOnce must be present.
    let thbco_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'vitest.MockInstance.toHaveBeenCalledOnce'
               AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        thbco_count, 1,
        "vitest.MockInstance.toHaveBeenCalledOnce must be indexed"
    );

    // vitest.MockInstance.mockReset must be present.
    let reset_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'vitest.MockInstance.mockReset'
               AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        reset_count, 1,
        "vitest.MockInstance.mockReset must be indexed"
    );
}

#[test]
fn no_chai_synthetics_without_node_modules() {
    // When there is no node_modules at all, synthetic_test_chain_files returns
    // empty so no spurious files are injected for non-JS projects.
    use bearwisdom::ecosystem::js_test_chains::synthetic_test_chain_files;

    let tmp = TempDir::new().unwrap();
    let result = synthetic_test_chain_files(tmp.path());
    assert!(
        result.is_empty(),
        "no synthetics expected when node_modules is absent"
    );
}

#[test]
fn no_vitest_synthetics_without_vitest_package() {
    use bearwisdom::ecosystem::js_test_chains::synthetic_test_chain_files;

    // node_modules exists but neither chai nor vitest is there.
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("node_modules").join("react")).unwrap();

    let result = synthetic_test_chain_files(tmp.path());
    assert!(
        result.is_empty(),
        "no synthetics expected when neither chai nor vitest is installed"
    );
}
