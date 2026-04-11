//! Integration test for S3 Go externals MVP.
//!
//! Seeds a temp directory that mimics `$GOMODCACHE/github.com/foo/bar@v1.0.0/`,
//! points GOMODCACHE at it, indexes a tiny Go project that imports the fake
//! package, and asserts the externals pipeline end-to-end:
//!
//!   1. External files land with origin='external'
//!   2. Internal origin filter hides externals from user queries
//!   3. External symbols participate in resolution (edges internal→external)
//!
//! This runs without a Go toolchain on the host — `$GOMODCACHE` is a plain
//! directory tree that we build from string literals.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// Build a synthetic `$GOMODCACHE` with a single package:
/// `github.com/fakeext/greeter@v1.0.0` exposing `Greet(name string) string`.
fn seed_fake_gomodcache() -> TempDir {
    let cache = TempDir::new().unwrap();
    let pkg_dir = cache
        .path()
        .join("github.com")
        .join("fakeext")
        .join("greeter@v1.0.0");
    fs::create_dir_all(&pkg_dir).unwrap();

    fs::write(
        pkg_dir.join("greeter.go"),
        r#"package greeter

// Greet returns a friendly salutation for the given name.
func Greet(name string) string {
    return "Hello, " + name
}

// Formatter is a re-usable greeting template.
type Formatter struct {
    Prefix string
}

func (f *Formatter) Format(name string) string {
    return f.Prefix + " " + name
}
"#,
    )
    .unwrap();

    cache
}

/// Build a tiny Go project that depends on the fake external package.
fn seed_consumer_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file(
        "go.mod",
        "module example.com/consumer\n\ngo 1.21\n\nrequire github.com/fakeext/greeter v1.0.0\n",
    );

    project.add_file(
        "main.go",
        r#"package main

import (
    "fmt"
    "github.com/fakeext/greeter"
)

func main() {
    msg := greeter.Greet("world")
    fmt.Println(msg)
}
"#,
    );

    project
}

#[test]
fn external_go_package_is_indexed_and_resolved() {
    let cache = seed_fake_gomodcache();
    let project = seed_consumer_project();

    // Temporarily point discovery at the synthetic cache. Single-threaded
    // because std::env::set_var is process-global.
    // Save/restore to avoid leaking into sibling tests.
    let prior_cache = std::env::var_os("GOMODCACHE");
    // SAFETY: cargo test is multi-threaded per default. We assume this is
    // the only test in this file; other test files don't read GOMODCACHE.
    unsafe {
        std::env::set_var("GOMODCACHE", cache.path());
    }

    let mut db = TestProject::in_memory_db();
    let stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    // Restore GOMODCACHE before assertions so a panic doesn't leak state.
    unsafe {
        match prior_cache {
            Some(v) => std::env::set_var("GOMODCACHE", v),
            None => std::env::remove_var("GOMODCACHE"),
        }
    }

    // --- Assertion 1: stats reflect internal-only counts ---
    assert!(
        stats.file_count >= 1 && stats.file_count <= 2,
        "expected 1-2 internal files (main.go, possibly go.mod), got {}",
        stats.file_count
    );
    assert!(
        stats.symbol_count >= 1,
        "expected at least one internal symbol (main), got {}",
        stats.symbol_count
    );

    // --- Assertion 2: external files actually landed in the DB ---
    let external_files: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_files >= 1,
        "expected at least one external file to be indexed, got {external_files}"
    );

    // --- Assertion 3: external symbols indexed ---
    let external_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_symbols >= 2,
        "expected Greet + Formatter (at least 2) external symbols, got {external_symbols}"
    );

    // --- Assertion 4: internal queries skip externals ---
    let internal_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE origin = 'internal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        internal_symbols >= 1,
        "expected at least 1 internal symbol, got {internal_symbols}"
    );

    // The externals must not appear in user-facing symbol search.
    let search_hits =
        bearwisdom::query::search::search_symbols(&db, "Greet", 10, &Default::default())
            .unwrap();
    assert!(
        search_hits.iter().all(|s| !s.qualified_name.contains("greeter")),
        "search_symbols leaked an external symbol: {:?}",
        search_hits.iter().map(|s| &s.qualified_name).collect::<Vec<_>>()
    );

    // --- Probe: does the resolver close the loop? ---
    // Does the `greeter.Greet` call from main.go become an edge targeting
    // the external Greet symbol, or does it remain in external_refs as an
    // opaque `ext:github.com/fakeext/greeter` row? This is the acid test
    // for Tier 1.5 loop closure.
    let edges_to_external: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.target_id
             WHERE s.origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    let greeter_external_refs: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM external_refs WHERE namespace LIKE 'ext:%greeter%'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert!(
        edges_to_external >= 1,
        "expected at least one internal→external edge (main.go calling greeter.Greet), got {edges_to_external}"
    );
    assert_eq!(
        greeter_external_refs, 0,
        "greeter should be resolved to real edges, not opaque external_refs rows"
    );
}
