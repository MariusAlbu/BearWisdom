//! Rust resolution probes — sibling to `resolution_corpus.rs`, same per-fix
//! harness shape: one in-memory `TestProject` indexed once through the real
//! pipeline, self-validating preconditions, asserted-green probes plus
//! printed known-red candidates carrying their traced root cause.
//!
//! `BEARWISDOM_RUST_SYSROOT` seeds a stub rust-src tree (mirrors
//! `BEARWISDOM_TS_LIB_DIR` in `resolution_corpus.rs`) and `CARGO_HOME` seeds
//! a stub Cargo registry crate (mirrors `BEARWISDOM_TS_NODE_MODULES`) so the
//! stdlib / external-crate probes run against pinned fixtures rather than
//! the machine's real rustup/cargo install.

use std::fs;

use bearwisdom::full_index;
use bearwisdom::Database;
use bearwisdom_tests::TestProject;
use rusqlite::params;
use tempfile::TempDir;

/// Count resolved `calls` edges for `callee` in the file ending `file_suffix`,
/// whose resolved target qname matches `target_like` (SQL LIKE).
fn count_resolved_to(db: &Database, file_suffix: &str, callee: &str, target_like: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         JOIN symbols t ON e.target_id = t.id
         WHERE f.path LIKE ?1 AND e.kind = 'calls' AND t.name = ?2
           AND t.qualified_name LIKE ?3",
        params![format!("%{file_suffix}"), callee, target_like],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count unresolved `calls` refs for `callee` in the file ending `file_suffix`.
fn count_unresolved(db: &Database, file_suffix: &str, callee: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM unresolved_refs u
         JOIN symbols s ON u.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         WHERE f.path LIKE ?1 AND u.kind = 'calls' AND u.target_name = ?2",
        params![format!("%{file_suffix}"), callee],
        |r| r.get(0),
    )
    .unwrap()
}

/// Seed a stub rust-src tree at `{root}/lib/rustlib/src/rust/library/{std,core}/src/`
/// carrying just the members the corpus references, so `String`/`Option` index
/// deterministically without a `rustup component add rust-src` install.
fn seed_rust_sysroot() -> TempDir {
    let dir = TempDir::new().unwrap();
    let std_src = dir.path().join("lib/rustlib/src/rust/library/std/src");
    let core_src = dir.path().join("lib/rustlib/src/rust/library/core/src");
    fs::create_dir_all(&std_src).unwrap();
    fs::create_dir_all(&core_src).unwrap();
    fs::write(
        std_src.join("string.rs"),
        r#"pub struct String;
impl String {
    pub fn new() -> String {
        String
    }
    pub fn len(&self) -> usize {
        0
    }
}
"#,
    )
    .unwrap();
    fs::write(
        core_src.join("option.rs"),
        r#"pub enum Option<T> {
    None,
    Some(T),
}
impl<T> Option<T> {
    pub fn unwrap(self) -> T {
        match self {
            Option::Some(t) => t,
            Option::None => panic!(),
        }
    }
}
"#,
    )
    .unwrap();
    dir
}

/// Seed a stub Cargo registry crate at
/// `{home}/registry/src/<index>/<name>-<ver>/src/lib.rs` so `use somecrate::Thing;`
/// resolves against a pinned external crate rather than the machine's real
/// `~/.cargo/registry`.
fn seed_cargo_registry() -> TempDir {
    let dir = TempDir::new().unwrap();
    let crate_src = dir
        .path()
        .join("registry/src/index.crates.io/somecrate-0.1.0/src");
    fs::create_dir_all(&crate_src).unwrap();
    fs::write(
        crate_src.join("lib.rs"),
        r#"pub struct Thing;
impl Thing {
    pub fn new() -> Thing {
        Thing
    }
    pub fn greet(&self) -> &str {
        "hi"
    }
}
"#,
    )
    .unwrap();
    dir
}

#[test]
fn resolution_corpus_rust() {
    let sysroot = seed_rust_sysroot();
    let cargo_home = seed_cargo_registry();

    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file(
        "Cargo.toml",
        r#"[package]
name = "resolution-corpus-rust"
version = "0.0.1"
edition = "2021"

[dependencies]
somecrate = "0.1.0"
"#,
    );
    // Registry-source dep with no path/git override, matching the shape
    // `discover_cargo_roots` requires to resolve against a registry root.
    project.add_file(
        "Cargo.lock",
        r#"version = 3

[[package]]
name = "somecrate"
version = "0.1.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#,
    );

    // --- pattern: same-file struct member, UFCS assoc-fn call syntax --------
    // `Index::exists(idx, d)` — type-qualified call syntax, not `idx.exists(d)`.
    project.add_file(
        "src/assoc_call.rs",
        r#"pub struct Doc;

pub struct Index;

impl Index {
    pub fn exists(&self, d: &Doc) -> bool {
        false
    }
}

pub fn check(idx: &Index, d: &Doc) -> bool {
    Index::exists(idx, d)
}
"#,
    );

    // --- pattern: generic type-arg grounding through a `?`-unwrapped local ---
    // `create()` returns `crate::Result<Segment>`; `create()?` should ground
    // the local `seg` on `Segment`, not leave it typed as the unpeeled
    // `Result<Segment>` wrapper.
    project.add_file(
        "src/result_unwrap.rs",
        r#"pub struct Segment;

impl Segment {
    pub fn exists(&self) -> bool {
        false
    }
}

pub type Result<T> = std::result::Result<T, String>;

pub fn create() -> crate::Result<Segment> {
    Ok(Segment)
}

pub fn use_segment() -> crate::Result<bool> {
    let seg = create()?;
    Ok(seg.exists())
}
"#,
    );

    // --- pattern: method on a local typed via a constructor call ------------
    project.add_file(
        "src/local_ctor.rs",
        r#"pub struct IndexWriter;

impl IndexWriter {
    pub fn new() -> IndexWriter {
        IndexWriter
    }

    pub fn commit(&mut self) -> bool {
        true
    }
}

pub fn build() -> bool {
    let mut w = IndexWriter::new();
    w.commit()
}
"#,
    );

    // --- pattern: std/prelude member calls against the seeded rust-src stub -
    project.add_file(
        "src/std_string.rs",
        r#"pub fn make() -> usize {
    let s = String::new();
    s.len()
}
"#,
    );
    project.add_file(
        "src/std_option.rs",
        r#"pub fn make() -> Option<i32> {
    Some(1)
}

pub fn use_it() -> i32 {
    let opt = make();
    opt.unwrap()
}
"#,
    );

    // --- pattern: use-imported external crate type via a seeded registry ----
    project.add_file(
        "src/external_crate.rs",
        r#"use somecrate::Thing;

pub fn greet_it() -> bool {
    let t = Thing::new();
    t.greet().is_empty()
}
"#,
    );

    // Point the locators at the seeded stubs, index once, restore env.
    let prior_sysroot = std::env::var_os("BEARWISDOM_RUST_SYSROOT");
    let prior_cargo_home = std::env::var_os("CARGO_HOME");
    unsafe {
        std::env::set_var("BEARWISDOM_RUST_SYSROOT", sysroot.path());
        std::env::set_var("CARGO_HOME", cargo_home.path());
    }

    let mut db = TestProject::in_memory_db();
    let result = full_index(&mut db, project.path(), None, None, None);

    unsafe {
        match prior_sysroot {
            Some(v) => std::env::set_var("BEARWISDOM_RUST_SYSROOT", v),
            None => std::env::remove_var("BEARWISDOM_RUST_SYSROOT"),
        }
        match prior_cargo_home {
            Some(v) => std::env::set_var("CARGO_HOME", v),
            None => std::env::remove_var("CARGO_HOME"),
        }
    }
    result.expect("index failed");

    // Preconditions: the stub externals MUST be indexed, else the patterns
    // that depend on them (stdlib boxing, external-crate pick) pass vacuously.
    let stub_string: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name='String' AND kind='struct' AND origin='external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let stub_option: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name='Option' AND origin='external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let stub_thing: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name='Thing' AND origin='external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("\n--- preconditions ---");
    println!("  stub std::String indexed (BEARWISDOM_RUST_SYSROOT): {stub_string}");
    println!("  stub core::Option indexed (BEARWISDOM_RUST_SYSROOT): {stub_option}");
    println!("  stub external crate Thing indexed (CARGO_HOME): {stub_thing}");

    // Candidate probes — KNOWN-RED, diagnostic only (not asserted). Root
    // causes (traced):
    //   result-unwrap — `create()?` records the try-expression on the flow
    //     cache (`rust_lang/flow.rs`'s `@rhs_unwrap` capture), but
    //     `flow_binding_unwrap` is read only by `canonical_form.rs`'s bounds
    //     check, never by `pipeline.rs`'s local-type seeding (which peels
    //     `flow_binding_await` but not `flow_binding_unwrap`). `seg` seeds as
    //     the unpeeled `Result<Segment>` (head "Result"), which has no
    //     `exists` member — `Segment` does, one unwrap layer down.
    //   std-option — `impl<T> Option<T> { fn unwrap(...) }`'s qualifying
    //     prefix is the impl target node's raw text: `extract_impl`
    //     (`rust_lang/calls.rs:41-45`) sets `type_name =
    //     node_text(&type_node, source)` for the `type` field directly,
    //     without reducing a `generic_type` node to its base name the way
    //     `rust_type_node_name()` does elsewhere. `unwrap` indexes as
    //     `Option<T>.unwrap`, not `Option.unwrap` — the member walk looks
    //     for the latter and misses. Unrelated to std-seeding (confirmed:
    //     the stub IS indexed); reproduces on any project-local
    //     `impl<T> Foo<T> { .. }` inherent-method block.
    println!("\n--- candidate probes (known red) ---");
    println!(
        "  result-unwrap  seg.exists() resolved-to-Segment={} unresolved={}",
        count_resolved_to(&db, "result_unwrap.rs", "exists", "%Segment%"),
        count_unresolved(&db, "result_unwrap.rs", "exists")
    );
    println!(
        "  std-option     opt.unwrap() resolved-to-Option={} unresolved={}",
        count_resolved_to(&db, "std_option.rs", "unwrap", "%Option%"),
        count_unresolved(&db, "std_option.rs", "unwrap")
    );

    // Each row: (label, pass, detail).
    let assoc_call_exists = count_resolved_to(&db, "assoc_call.rs", "exists", "%Index%");
    let local_ctor_commit = count_resolved_to(&db, "local_ctor.rs", "commit", "%IndexWriter%");
    let std_string_len = count_resolved_to(&db, "std_string.rs", "len", "%String%");
    let external_crate_greet = count_resolved_to(&db, "external_crate.rs", "greet", "%Thing%");

    let checks = [
        (
            "UFCS assoc call  Index::exists(idx, d) -> Index.exists",
            assoc_call_exists >= 1,
            format!("resolved-to-Index edges = {assoc_call_exists}"),
        ),
        (
            "local ctor       w.commit() -> IndexWriter.commit",
            local_ctor_commit >= 1,
            format!("resolved-to-IndexWriter edges = {local_ctor_commit}"),
        ),
        (
            "std seam (sysroot)  s.len() -> String.len (BEARWISDOM_RUST_SYSROOT)",
            std_string_len >= 1,
            format!("resolved-to-String edges = {std_string_len}"),
        ),
        (
            "external crate seam  t.greet() -> Thing.greet (CARGO_HOME)",
            external_crate_greet >= 1,
            format!("resolved-to-Thing edges = {external_crate_greet}"),
        ),
    ];

    println!("\n=== resolution corpus (rust) ===");
    let mut failures = Vec::new();
    for (label, pass, detail) in &checks {
        println!("  {} {label}  [{detail}]", if *pass { "✅" } else { "❌" });
        if !pass {
            failures.push(*label);
        }
    }
    println!(
        "  {} / {} patterns resolved as expected\n",
        checks.len() - failures.len(),
        checks.len()
    );

    assert!(
        failures.is_empty(),
        "resolution corpus (rust) regressions: {failures:?}"
    );
}
