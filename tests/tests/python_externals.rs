//! Integration test for S4 Python externals MVP.
//!
//! Mirrors `go_externals.rs`: seeds a fake site-packages with one package,
//! points `BEARWISDOM_PYTHON_SITE_PACKAGES` at it, indexes a consumer
//! project whose pyproject.toml depends on that package, and asserts the
//! full externals pipeline end-to-end.

use std::fs;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// Build a synthetic site-packages with one package: `fakelib` exposing
/// `Engine.connect()` + `create_engine(url)`.
fn seed_fake_site_packages() -> TempDir {
    let sp = TempDir::new().unwrap();
    let pkg = sp.path().join("fakelib");
    fs::create_dir_all(&pkg).unwrap();

    fs::write(
        pkg.join("__init__.py"),
        r#""""Fakelib — a tiny stand-in for a real Python package."""

from .engine import Engine, create_engine

__all__ = ["Engine", "create_engine"]
"#,
    )
    .unwrap();

    fs::write(
        pkg.join("engine.py"),
        r#""""Connection engine primitives."""


class Engine:
    """Represents a database connection pool."""

    def __init__(self, url: str) -> None:
        self.url = url

    def connect(self) -> "Connection":
        return Connection(self)


class Connection:
    def __init__(self, engine: Engine) -> None:
        self.engine = engine

    def execute(self, sql: str) -> None:
        pass


def create_engine(url: str) -> Engine:
    """Top-level factory that returns an Engine."""
    return Engine(url)
"#,
    )
    .unwrap();

    sp
}

/// Build a tiny Python project that depends on `fakelib`.
fn seed_consumer_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file(
        "pyproject.toml",
        r#"[project]
name = "consumer"
version = "0.1.0"
dependencies = [
    "fakelib>=1.0",
]
"#,
    );

    project.add_file(
        "app.py",
        r#"from fakelib import create_engine, Engine


def bootstrap(url: str) -> Engine:
    engine = create_engine(url)
    return engine


def main() -> None:
    eng = bootstrap("postgres://localhost/db")
    conn = eng.connect()
    conn.execute("SELECT 1")
"#,
    );

    project
}

#[test]
fn external_python_package_is_indexed_and_resolved() {
    let site_packages = seed_fake_site_packages();
    let project = seed_consumer_project();

    let prior = std::env::var_os("BEARWISDOM_PYTHON_SITE_PACKAGES");
    unsafe {
        std::env::set_var("BEARWISDOM_PYTHON_SITE_PACKAGES", site_packages.path());
    }

    let mut db = TestProject::in_memory_db();
    let stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    // Restore env before asserting so a panic doesn't leak state across tests.
    unsafe {
        match prior {
            Some(v) => std::env::set_var("BEARWISDOM_PYTHON_SITE_PACKAGES", v),
            None => std::env::remove_var("BEARWISDOM_PYTHON_SITE_PACKAGES"),
        }
    }

    // --- Internal stats reflect only the consumer project ---
    assert!(
        stats.file_count >= 1,
        "expected at least 1 internal file, got {}",
        stats.file_count
    );
    assert!(
        stats.symbol_count >= 1,
        "expected at least 1 internal symbol, got {}",
        stats.symbol_count
    );

    // --- External files landed ---
    let external_files: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_files >= 1,
        "expected at least 1 external file, got {external_files}"
    );

    // --- External symbols indexed ---
    let external_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_symbols >= 3,
        "expected Engine, Connection, create_engine (at least 3), got {external_symbols}"
    );

    // --- User queries skip externals ---
    let search_hits =
        bearwisdom::query::search::search_symbols(&db, "Engine", 10, &Default::default())
            .unwrap();
    assert!(
        search_hits.iter().all(|s| !s.qualified_name.contains("fakelib")),
        "search_symbols leaked an external symbol: {:?}",
        search_hits.iter().map(|s| &s.qualified_name).collect::<Vec<_>>()
    );

    // --- Tier 1 resolver closes the loop: internal→external edges exist ---
    let edges_to_external: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.target_id
             WHERE s.origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        edges_to_external >= 1,
        "expected at least one internal→external edge (app.py → fakelib.create_engine), got {edges_to_external}"
    );
}
