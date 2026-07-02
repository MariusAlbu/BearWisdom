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

pub struct SelfProbe;
impl SelfProbe {
    pub fn new() -> SelfProbe {
        SelfProbe
    }
    pub fn external_marker(&self) -> bool {
        false
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

[workspace]
members = ["member"]
"#,
    );
    // A real (if trivial) workspace member alongside the hybrid root, matching
    // the shape `tantivy` / `loco-rs` actually ship: a root `Cargo.toml` that
    // carries both its own `[package]` and a `[workspace]` table.
    project.add_file(
        "member/Cargo.toml",
        r#"[package]
name = "resolution-corpus-rust-member"
version = "0.0.1"
edition = "2021"
"#,
    );
    project.add_file("member/src/lib.rs", "pub struct MemberPlaceholder;\n");
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

    // --- pattern: intermediate call in a member chain, no local binding -----
    // `Builder::new().build()` — `new` is a nested call whose RETURN type
    // (`Builder`) the walker must follow to look up `build`. `.build().show()`
    // chains a second intermediate call whose return (`Widget`) gates `show`.
    project.add_file(
        "src/chain_call.rs",
        r#"pub struct Builder;

impl Builder {
    pub fn new() -> Builder {
        Builder
    }

    pub fn build(&self) -> Widget {
        Widget
    }
}

pub struct Widget;

impl Widget {
    pub fn show(&self) -> bool {
        true
    }
}

pub fn make() -> Widget {
    Builder::new().build()
}

pub fn make_and_show() -> bool {
    Builder::new().build().show()
}
"#,
    );

    // --- pattern: bench target imports its own crate by published name ------
    // `benches/`, `examples/`, and `tests/` targets compile as separate crates
    // that reach the library through `use <crate_name>::X` — the exact same
    // syntax an external consumer would use, since Cargo has no "internal"
    // import form for them.
    //
    // `SelfProbe` is deliberately name-colliding with `somecrate::SelfProbe`
    // (seeded in `seed_cargo_registry`, method `external_marker`) at BOTH the
    // crate root and one nested module (`selfmod`) — a globally unique name
    // would resolve through the same bare-qname fallback that already binds
    // same-file structs (see `assoc_call.rs` below), without ever exercising
    // import-scoped disambiguation. With the collision, only a bind that
    // actually reads the `use` specifier can land on the project's own
    // `touch`/`poke` methods instead of the external stub's `external_marker`.
    project.add_file(
        "src/self_import.rs",
        r#"pub struct SelfProbe;

impl SelfProbe {
    pub fn new() -> SelfProbe {
        SelfProbe
    }

    pub fn touch(&self) -> bool {
        true
    }
}

pub mod selfmod {
    pub struct SelfProbe;

    impl SelfProbe {
        pub fn new() -> SelfProbe {
            SelfProbe
        }

        pub fn poke(&self) -> bool {
            true
        }
    }
}
"#,
    );
    project.add_file(
        "benches/bench_self_import.rs",
        r#"use resolution_corpus_rust::SelfProbe;

pub fn run_bench_flat() -> bool {
    let p = SelfProbe::new();
    p.touch()
}
"#,
    );
    project.add_file(
        "benches/bench_self_import_nested.rs",
        r#"use resolution_corpus_rust::selfmod::SelfProbe;

pub fn run_bench_nested() -> bool {
    let p = SelfProbe::new();
    p.poke()
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
    println!("\n--- candidate probes (known red) ---");
    println!(
        "  result-unwrap  seg.exists() resolved-to-Segment={} unresolved={}",
        count_resolved_to(&db, "result_unwrap.rs", "exists", "%Segment%"),
        count_unresolved(&db, "result_unwrap.rs", "exists")
    );

    // Each row: (label, pass, detail).
    let assoc_call_exists = count_resolved_to(&db, "assoc_call.rs", "exists", "%Index%");
    let local_ctor_commit = count_resolved_to(&db, "local_ctor.rs", "commit", "%IndexWriter%");
    let std_string_len = count_resolved_to(&db, "std_string.rs", "len", "%String%");
    let std_option_unwrap = count_resolved_to(&db, "std_option.rs", "unwrap", "%Option%");
    let external_crate_greet = count_resolved_to(&db, "external_crate.rs", "greet", "%Thing%");
    let chain_call_build = count_resolved_to(&db, "chain_call.rs", "build", "%Builder%");
    let chain_call_show = count_resolved_to(&db, "chain_call.rs", "show", "%Widget%");
    let self_import_flat_touch =
        count_resolved_to(&db, "bench_self_import.rs", "touch", "%SelfProbe%");
    let self_import_nested_poke =
        count_resolved_to(&db, "bench_self_import_nested.rs", "poke", "%SelfProbe%");

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
            "std seam (sysroot)  opt.unwrap() -> Option.unwrap (BEARWISDOM_RUST_SYSROOT)",
            std_option_unwrap >= 1,
            format!("resolved-to-Option edges = {std_option_unwrap}"),
        ),
        (
            "external crate seam  t.greet() -> Thing.greet (CARGO_HOME)",
            external_crate_greet >= 1,
            format!("resolved-to-Thing edges = {external_crate_greet}"),
        ),
        (
            "chain call (r6a)  Builder::new().build() -> Builder.build",
            chain_call_build >= 1,
            format!("resolved-to-Builder edges = {chain_call_build}"),
        ),
        (
            "chain call (r6b)  Builder::new().build().show() -> Widget.show",
            chain_call_show >= 1,
            format!("resolved-to-Widget edges = {chain_call_show}"),
        ),
        (
            "self-crate import (bench, flat)  SelfProbe::new().touch() -> self_import.SelfProbe.touch",
            self_import_flat_touch >= 1,
            format!("resolved-to-SelfProbe(root).touch edges = {self_import_flat_touch}"),
        ),
        (
            "self-crate import (bench, nested module)  SelfProbe::new().poke() -> selfmod.SelfProbe.poke",
            self_import_nested_poke >= 1,
            format!("resolved-to-SelfProbe(nested).poke edges = {self_import_nested_poke}"),
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
