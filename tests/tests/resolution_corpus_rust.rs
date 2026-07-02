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

/// Count unresolved refs of `kind` targeting `target_name` in the file ending
/// `file_suffix`.
fn count_unresolved(db: &Database, file_suffix: &str, kind: &str, target_name: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM unresolved_refs u
         JOIN symbols s ON u.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         WHERE f.path LIKE ?1 AND u.kind = ?2 AND u.target_name = ?3",
        params![format!("%{file_suffix}"), kind, target_name],
        |r| r.get(0),
    )
    .unwrap()
}

/// Count resolved `calls` edges for `callee` in the file ending `file_suffix`,
/// whose resolved target has the given `origin` (`'internal'` | `'external'`).
fn count_resolved_with_origin(db: &Database, file_suffix: &str, callee: &str, origin: &str) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON e.source_id = s.id
         JOIN files f   ON s.file_id   = f.id
         JOIN symbols t ON e.target_id = t.id
         WHERE f.path LIKE ?1 AND e.kind = 'calls' AND t.name = ?2
           AND t.origin = ?3",
        params![format!("%{file_suffix}"), callee, origin],
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
    // Real rust-src layout, mirrored exactly: `core/src/macros/mod.rs` opens
    // with a macro-2.0 (`decl_macro`) `pub macro assert_matches { ... }`
    // definition tree-sitter-rust 0.24 has no grammar node for, and the
    // real `assert!` is a `#[macro_export]` `macro_rules!` nested inside an
    // internal `pub(crate) mod builtin { ... }` used purely for source
    // organization — `#[macro_export]` hoists it to the crate root despite
    // the nesting.
    let core_macros = core_src.join("macros");
    fs::create_dir_all(&core_macros).unwrap();
    fs::write(
        core_macros.join("mod.rs"),
        r#"pub macro assert_matches {
    ($left:expr, $right:pat) => {
        match $left {
            $right => {}
            _ => panic!(),
        }
    },
}

pub(crate) mod builtin {
    #[macro_export]
    macro_rules! assert {
        ($cond:expr $(,)?) => {
            if !$cond {
                panic!("assertion failed");
            }
        };
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

pub struct GlobProbe;
impl GlobProbe {
    pub fn new() -> GlobProbe {
        GlobProbe
    }
    pub fn external_marker(&self) -> bool {
        false
    }
}

pub struct AliasDoc;
impl AliasDoc {
    pub fn new() -> AliasDoc {
        AliasDoc
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

    // --- pattern: bare std macro call against the seeded rust-src stub ------
    // `assert!(x)` — a macro invocation with no receiver, no chain, no `use`.
    // Macros are code with real targets: rust-src ships `assert!` as a
    // `macro_rules!` definition in `core/src/macros/mod.rs` (seeded above),
    // and this call must bind to it the same way any other prelude symbol
    // does — through the ambient stdlib surface, not a drain.
    project.add_file(
        "src/std_macros.rs",
        r#"pub fn check_it(x: bool) {
    assert!(x);
}
"#,
    );

    // --- pattern: project-local macro_rules!, invoked from another module ---
    // `my_thing!()` — a macro defined in one file and invoked from a sibling
    // module via an explicit `use crate::macro_def::my_thing;` import, the
    // same shape an ordinary cross-module function call already resolves
    // through.
    project.add_file(
        "src/macro_def.rs",
        r#"#[macro_export]
macro_rules! my_thing {
    () => {
        true
    };
}
"#,
    );
    project.add_file(
        "src/macro_call.rs",
        r#"use crate::macro_def::my_thing;

pub fn call_it() -> bool {
    my_thing!()
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

    // --- pattern: bench target glob-imports its own crate by published name --
    // `use tantivy::collector::*;` — every name the target module's public
    // surface exposes becomes bare-scoped, including ones reached only through
    // a `pub use` re-export nested deeper than the glob's own module path.
    // `GlobProbe` collides with a same-named `somecrate::GlobProbe` (seeded in
    // `seed_cargo_registry`) for the same reason `SelfProbe` does above: a
    // globally unique name would resolve through the bare-qname fallback
    // without ever exercising the glob-scoped bind.
    project.add_file(
        "src/glob_probe.rs",
        r#"pub struct GlobProbe;

impl GlobProbe {
    pub fn new() -> GlobProbe {
        GlobProbe
    }

    pub fn touch(&self) -> bool {
        true
    }
}
"#,
    );
    project.add_file(
        "benches/bench_glob_import.rs",
        r#"use resolution_corpus_rust::*;

pub fn run_bench_glob() -> bool {
    let p = GlobProbe::new();
    p.touch()
}
"#,
    );
    // --- pattern: bench target glob-imports a NESTED module of its own crate -
    // `use tantivy::collector::*;`'s multi-segment shape: the glob's module path
    // carries a sub-path (`inner`) that must filter candidates by file, not just
    // by package. `decoy_mod.rs` declares a same-named `InnerProbe` OUTSIDE the
    // `inner` module so a bind that ignores the sub-path (package-wide, first
    // same-name hit) cannot pass by accident.
    project.add_file(
        "src/inner.rs",
        r#"pub struct InnerProbe;

impl InnerProbe {
    pub fn new() -> InnerProbe {
        InnerProbe
    }

    pub fn ping(&self) -> bool {
        true
    }
}
"#,
    );
    project.add_file(
        "src/decoy_mod.rs",
        r#"pub struct InnerProbe;

impl InnerProbe {
    pub fn new() -> InnerProbe {
        InnerProbe
    }

    pub fn wrong(&self) -> bool {
        false
    }
}
"#,
    );
    project.add_file(
        "benches/bench_glob_import_nested.rs",
        r#"use resolution_corpus_rust::inner::*;

pub fn run_bench_glob_nested() -> bool {
    let p = InnerProbe::new();
    p.ping()
}
"#,
    );

    // --- pattern: crate::-relative self-reference through a pub-use re-export
    // `Thing` is declared in `src/thing.rs` and re-exported at the crate root
    // (`pub use thing::Thing;` in `lib.rs`). `crate_reexport.rs` names it via
    // the re-exported crate-root path (`use crate::Thing;`); `crate_direct.rs`
    // names it via its declaring submodule path (`use crate::thing::Thing;`).
    // The synthetic TypeRef that seeds `t`'s local type from `let t =
    // Thing::new();` carries this import's module (`crate` / `crate::thing`)
    // — an unrecognized specifier leaves that TypeRef unresolved
    // (`cause=unbound_root`) even on the constructor-call shape where the
    // chain walker's own root/call resolution happens to re-derive `t`'s type
    // and rescue the downstream `.go()` member lookup. The unresolved TypeRef
    // itself is the count-inflating symptom this probe targets.
    project.add_file(
        "src/thing.rs",
        r#"pub struct Thing;

impl Thing {
    pub fn new() -> Thing {
        Thing
    }

    pub fn go(&self) -> bool {
        true
    }
}
"#,
    );
    project.add_file(
        "src/lib.rs",
        r#"mod thing;
pub use thing::Thing;
mod glob_probe;
pub use glob_probe::GlobProbe;
pub mod inner;
mod real;
pub use real::RealDoc as AliasDoc;
"#,
    );
    project.add_file(
        "src/crate_reexport.rs",
        r#"use crate::Thing;

pub fn run_reexported() -> bool {
    let t = Thing::new();
    t.go()
}
"#,
    );
    project.add_file(
        "src/crate_direct.rs",
        r#"use crate::thing::Thing;

pub fn run_direct() -> bool {
    let t = Thing::new();
    t.go()
}
"#,
    );
    // Canonical real-world shape: a `#[cfg(test)]` sibling module inside a
    // `src/` file, importing its own crate's type by the `crate::` path
    // exactly like tantivy's `src/aggregation/agg_tests.rs`.
    project.add_file(
        "src/cfg_test_reexport.rs",
        r#"#[cfg(test)]
mod tests {
    use crate::Thing;

    #[test]
    fn it_works() {
        let t = Thing::new();
        let _ = t.go();
    }
}
"#,
    );

    // --- pattern: Vec<T> element projection at a subscript receiver --------
    // `v[0].touch()` — `v`'s type is `Vec<Item>` (via `make_items`'s return,
    // not a local annotation — an angle-bracket-generic annotation's argument
    // is stripped before it reaches the type interner, a separate gap traced
    // below). The `[0]` subscript must project the ELEMENT type `Item` so
    // `touch` resolves there, not against `Vec` (which has no `touch`).
    // `Decoy` declares a same-named `touch` FIRST in the file so a same-file,
    // name-only fallback (which would win if the chain walker never types the
    // receiver — e.g. because the subscript breaks chain-building entirely)
    // resolves to the WRONG target; only a receiver-typed bind lands on `Item`.
    project.add_file(
        "src/vec_subscript.rs",
        r#"pub struct Decoy;

impl Decoy {
    pub fn touch(&self) -> bool {
        false
    }
}

pub struct Item;

impl Item {
    pub fn touch(&self) -> bool {
        true
    }
}

pub fn make_items() -> Vec<Item> {
    Vec::new()
}

pub fn use_vec() -> bool {
    let v = make_items();
    v[0].touch()
}
"#,
    );

    // --- pattern: Vec<T> element projection through a struct field ---------
    // `list.segments[0].touch()` — the field's declared type (`Vec<Segment>`)
    // is captured whole (struct field signatures aren't run through the
    // lossy local-annotation path), so this exercises element projection
    // through a field member lookup rather than a bare local. `list` is typed
    // via `make_list`'s return (not `self` — a `self.field` receiver hits a
    // separate, pre-existing gap: an impl-block method's `parent_index` never
    // climbs to the struct it implements, since the impl block's own Namespace
    // symbol carries `parent_index: None`, so `enclosing_type_qname` can never
    // find it from a method; out of scope here). `Decoy` guards against the
    // same-file fallback as above.
    project.add_file(
        "src/vec_field_subscript.rs",
        r#"pub struct Decoy;

impl Decoy {
    pub fn touch(&self) -> bool {
        false
    }
}

pub struct Segment;

impl Segment {
    pub fn touch(&self) -> bool {
        true
    }
}

pub struct SegmentList {
    pub segments: Vec<Segment>,
}

pub fn make_list() -> SegmentList {
    SegmentList { segments: Vec::new() }
}

pub fn first_touch() -> bool {
    let list = make_list();
    list.segments[0].touch()
}
"#,
    );

    // --- pattern: Vec<T> element projection through a `self` field, inside
    // the implementing impl block --------------------------------------
    // `self.segments[0].touch()` — `self` roots on the enclosing impl's
    // target type via the scope-chain / scope-path fallback, since methods
    // extracted as impl-block siblings carry no `parent_index` chain up to
    // their struct. `Decoy` guards against a same-name fallback resolving
    // the call by luck rather than by the receiver's actual element type.
    project.add_file(
        "src/vec_self_field_subscript.rs",
        r#"pub struct Decoy;

impl Decoy {
    pub fn touch(&self) -> bool {
        false
    }
}

pub struct SelfSegment;

impl SelfSegment {
    pub fn touch(&self) -> bool {
        true
    }
}

pub struct SelfSegmentList {
    pub segments: Vec<SelfSegment>,
}

impl SelfSegmentList {
    pub fn first_touch_self(&self) -> bool {
        self.segments[0].touch()
    }
}
"#,
    );

    // --- pattern: `self.field.method()` with no subscript in the chain -----
    // The simplest shape that depends on the same `self`-rooting fallback:
    // one field hop, one call, no bracket projection involved. `Decoy` guards
    // against a same-name fallback resolving `mark()` by luck.
    project.add_file(
        "src/self_field_method.rs",
        r#"pub struct Decoy;

impl Decoy {
    pub fn mark(&self) -> bool {
        false
    }
}

pub struct Namer;

impl Namer {
    pub fn mark(&self) -> bool {
        true
    }
}

pub struct Cfg {
    pub name: Namer,
}

impl Cfg {
    pub fn go(&self) -> bool {
        self.name.mark()
    }
}
"#,
    );

    // --- pattern: fixed-size array element projection at a subscript -------
    // `a[0].touch()` — `a: [Elem; 2]` must project the element `Elem` the
    // same way a `Vec<Elem>` subscript does. Unlike `Vec<T>`, the bracket
    // annotation carries no `<` so it isn't truncated by the angle-bracket
    // generic-arg strip — the local's declared type reaches the interner
    // whole. `Decoy` guards against the same-file fallback as above.
    project.add_file(
        "src/array_subscript.rs",
        r#"pub struct Decoy;

impl Decoy {
    pub fn touch(&self) -> bool {
        false
    }
}

pub struct Elem;

impl Elem {
    pub fn touch(&self) -> bool {
        true
    }
}

pub fn use_array() -> bool {
    let a: [Elem; 2] = [Elem, Elem];
    a[0].touch()
}
"#,
    );

    // --- pattern: slice element projection at a subscript -------------------
    // `s[0].touch()` — `s: &'static [Piece]` (via `make_slice`'s return) must
    // project the element `Piece` the same way the array/Vec cases do.
    // `Decoy` guards against the same-file fallback as above.
    project.add_file(
        "src/slice_subscript.rs",
        r#"pub struct Decoy;

impl Decoy {
    pub fn touch(&self) -> bool {
        false
    }
}

pub struct Piece;

impl Piece {
    pub fn touch(&self) -> bool {
        true
    }
}

pub fn make_slice() -> &'static [Piece] {
    &[Piece, Piece]
}

pub fn use_slice() -> bool {
    let s = make_slice();
    s[0].touch()
}
"#,
    );

    // --- pattern: Vec<T> element projection through an annotated local, no
    // resolvable RHS to rescue it -------------------------------------------
    // `let v: Vec<Item> = Vec::new(); v[0].touch()` — the LHS annotation is
    // the only place `Item` appears in source, so `v`'s recorded type must
    // carry the full `Vec<Item>` text for the subscript to project the
    // element. `Decoy` guards against the same-file fallback as above.
    project.add_file(
        "src/vec_annotation_only.rs",
        r#"pub struct Decoy;

impl Decoy {
    pub fn touch(&self) -> bool {
        false
    }
}

pub struct Widget;

impl Widget {
    pub fn touch(&self) -> bool {
        true
    }
}

pub fn use_annotated_vec() -> bool {
    let v: Vec<Widget> = Vec::new();
    v[0].touch()
}
"#,
    );

    // --- pattern: renamed re-export, `pub use path::Thing as Alias;` ---------
    // `RealDoc` is declared in `src/real.rs` and re-exported under a different
    // name at the crate root (`pub use real::RealDoc as AliasDoc;` in
    // `lib.rs`) — the shape tantivy's `pub use CompactDoc as TantivyDocument;`
    // takes. `AliasDoc` collides with a same-named `somecrate::AliasDoc`
    // (seeded in `seed_cargo_registry`, method `external_marker`) for the same
    // reason `SelfProbe`/`GlobProbe` do above: a bind that resolves purely by
    // the alias name without carrying it to `RealDoc`'s own members would find
    // no `touch` method on either candidate, so the probe can't pass by
    // accident.
    project.add_file(
        "src/real.rs",
        r#"pub struct RealDoc;

impl RealDoc {
    pub fn new() -> RealDoc {
        RealDoc
    }

    pub fn touch(&self) -> bool {
        true
    }
}
"#,
    );
    project.add_file(
        "benches/bench_alias_reexport.rs",
        r#"use resolution_corpus_rust::AliasDoc;

pub fn run_bench_alias() -> bool {
    let p = AliasDoc::new();
    p.touch()
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
    let stub_assert_macro: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE name='assert' AND kind='function' AND origin='external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!("\n--- preconditions ---");
    println!("  stub std::String indexed (BEARWISDOM_RUST_SYSROOT): {stub_string}");
    println!("  stub core::Option indexed (BEARWISDOM_RUST_SYSROOT): {stub_option}");
    println!("  stub external crate Thing indexed (CARGO_HOME): {stub_thing}");
    println!("  stub core::macros::assert! indexed (BEARWISDOM_RUST_SYSROOT): {stub_assert_macro}");

    // Candidate probes — KNOWN-RED, diagnostic only (not asserted). Root
    // causes (traced):
    //   result-unwrap — `create()?` records the try-expression on the flow
    //     cache (`rust_lang/flow.rs`'s `@rhs_unwrap` capture), but
    //     `flow_binding_unwrap` is read only by `canonical_form.rs`'s bounds
    //     check, never by `pipeline.rs`'s local-type seeding (which peels
    //     `flow_binding_await` but not `flow_binding_unwrap`). `seg` seeds as
    //     the unpeeled `Result<Segment>` (head "Result"), which has no
    //     `exists` member — `Segment` does, one unwrap layer down.
    //   local-macro-import — `my_thing!()` is a bare single-`identifier`
    //     call, and `build_chain` (`rust_lang/calls.rs:1006-1007`) returns
    //     `None` for any bare identifier — true for `macro_invocation` calls
    //     AND ordinary free-function calls alike. The Third-pass import-map
    //     enrichment (`rust_lang/extract.rs:104-121`) that copies a `use`d
    //     module onto a bare Calls ref only fires when `chain.segments.len()
    //     >= 2`, so it never runs here — the ref keeps `module: None` even
    //     though the file's own `use crate::macro_def::my_thing;` names the
    //     defining module. Not macro-specific: any bare, `use`-imported,
    //     unqualified call (macro or free function) hits the same gap.
    //   alias-reexport-member-chase — `AliasDoc::new().touch()` needs
    //     `AliasDoc` (the synthetic symbol `calls_imports.rs`'s `use_as_clause`
    //     arm registers for the alias) to carry `RealDoc`'s members. Rust
    //     never populates `ParsedFile::alias_targets` (hardcoded empty in
    //     `extract.rs`), so the generic `AliasTarget`/`expand_alias` machinery
    //     that would redirect a `TypeAlias`-kind symbol to its target has
    //     nothing to expand for Rust — the same gap TS has for a true rename
    //     (`export { X as Y } from './m'`): its synthetic `Y` symbol carries
    //     no type info either, so a chain through `Y` cannot reach `X`'s
    //     members. Wiring `alias_targets` for Rust is separate, larger work
    //     than re-export addressability.
    println!("\n--- candidate probes (known red) ---");
    println!(
        "  result-unwrap  seg.exists() resolved-to-Segment={} unresolved={}",
        count_resolved_to(&db, "result_unwrap.rs", "exists", "%Segment%"),
        count_unresolved(&db, "result_unwrap.rs", "calls", "exists")
    );
    println!(
        "  local-macro-import  my_thing!() resolved-internal={} unresolved={}",
        count_resolved_with_origin(&db, "macro_call.rs", "my_thing", "internal"),
        count_unresolved(&db, "macro_call.rs", "calls", "my_thing")
    );
    println!(
        "  alias-reexport-member-chase  AliasDoc::new().touch() resolved-to-RealDoc={} unresolved={}",
        count_resolved_to(&db, "bench_alias_reexport.rs", "touch", "%RealDoc%"),
        count_unresolved(&db, "bench_alias_reexport.rs", "calls", "touch")
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
    // The `.touch()` / `.ping()` calls below already resolve regardless of this
    // gap — the chain walker's own root/call re-derivation rescues them the
    // same way it rescues `crate_reexport.rs`'s `.go()` (see that pattern's
    // comment). The synthetic TypeRef that seeds `p`'s local type from
    // `GlobProbe::new()` / `InnerProbe::new()` is the one that dies on
    // `cause=unbound_root` without the glob-scoped bind — that TypeRef is the
    // count-inflating symptom these probes target.
    let glob_import_globprobe_unresolved =
        count_unresolved(&db, "bench_glob_import.rs", "type_ref", "GlobProbe");
    let glob_import_nested_innerprobe_unresolved =
        count_unresolved(&db, "bench_glob_import_nested.rs", "type_ref", "InnerProbe");
    let std_macro_assert =
        count_resolved_with_origin(&db, "std_macros.rs", "assert", "external");
    let crate_reexport_thing_unresolved =
        count_unresolved(&db, "crate_reexport.rs", "type_ref", "Thing");
    let crate_direct_thing_unresolved =
        count_unresolved(&db, "crate_direct.rs", "type_ref", "Thing");
    let cfg_test_reexport_thing_unresolved =
        count_unresolved(&db, "cfg_test_reexport.rs", "type_ref", "Thing");
    let vec_subscript_touch = count_resolved_to(&db, "vec_subscript.rs", "touch", "%Item%");
    let vec_field_subscript_touch =
        count_resolved_to(&db, "vec_field_subscript.rs", "touch", "%Segment%");
    let array_subscript_touch = count_resolved_to(&db, "array_subscript.rs", "touch", "%Elem%");
    let slice_subscript_touch = count_resolved_to(&db, "slice_subscript.rs", "touch", "%Piece%");
    let vec_annotation_only_touch =
        count_resolved_to(&db, "vec_annotation_only.rs", "touch", "%Widget%");
    let alias_reexport_aliasdoc_unresolved =
        count_unresolved(&db, "bench_alias_reexport.rs", "type_ref", "AliasDoc");
    let vec_self_field_subscript_touch =
        count_resolved_to(&db, "vec_self_field_subscript.rs", "touch", "%SelfSegment%");
    let self_field_method_mark = count_resolved_to(&db, "self_field_method.rs", "mark", "%Namer%");

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
        (
            "self-crate glob import (bench, crate root)  use resolution_corpus_rust::*; GlobProbe local-type TypeRef binds (not unbound_root)",
            glob_import_globprobe_unresolved == 0,
            format!("unresolved type_ref(GlobProbe) = {glob_import_globprobe_unresolved}"),
        ),
        (
            "self-crate glob import (bench, nested module)  use resolution_corpus_rust::inner::*; InnerProbe local-type TypeRef binds (not unbound_root)",
            glob_import_nested_innerprobe_unresolved == 0,
            format!("unresolved type_ref(InnerProbe) = {glob_import_nested_innerprobe_unresolved}"),
        ),
        (
            "std macro (sysroot)  assert!(x) -> core::macros::assert (BEARWISDOM_RUST_SYSROOT)",
            std_macro_assert >= 1,
            format!("resolved-to-external-assert edges = {std_macro_assert}"),
        ),
        (
            "crate::-relative re-export  use crate::Thing; local-type TypeRef binds (not unbound_root)",
            crate_reexport_thing_unresolved == 0,
            format!("unresolved type_ref(Thing) = {crate_reexport_thing_unresolved}"),
        ),
        (
            "crate::-relative direct  use crate::thing::Thing; local-type TypeRef binds (not unbound_root)",
            crate_direct_thing_unresolved == 0,
            format!("unresolved type_ref(Thing) = {crate_direct_thing_unresolved}"),
        ),
        (
            "crate::-relative re-export in #[cfg(test)] sibling module  local-type TypeRef binds",
            cfg_test_reexport_thing_unresolved == 0,
            format!("unresolved type_ref(Thing) = {cfg_test_reexport_thing_unresolved}"),
        ),
        (
            "Vec<T> subscript (local, call-return)  v[0].touch() -> Item.touch",
            vec_subscript_touch >= 1,
            format!("resolved-to-Item edges = {vec_subscript_touch}"),
        ),
        (
            "Vec<T> subscript (struct field)  list.segments[0].touch() -> Segment.touch",
            vec_field_subscript_touch >= 1,
            format!("resolved-to-Segment edges = {vec_field_subscript_touch}"),
        ),
        (
            "fixed-array subscript (annotated local)  a[0].touch() -> Elem.touch",
            array_subscript_touch >= 1,
            format!("resolved-to-Elem edges = {array_subscript_touch}"),
        ),
        (
            "slice subscript (call-return)  s[0].touch() -> Piece.touch",
            slice_subscript_touch >= 1,
            format!("resolved-to-Piece edges = {slice_subscript_touch}"),
        ),
        (
            "Vec<T> subscript (annotated local, no resolvable RHS)  v[0].touch() -> Widget.touch",
            vec_annotation_only_touch >= 1,
            format!("resolved-to-Widget edges = {vec_annotation_only_touch}"),
        ),
        (
            "renamed re-export (bench)  use resolution_corpus_rust::AliasDoc; local-type TypeRef binds (not unbound_root)",
            alias_reexport_aliasdoc_unresolved == 0,
            format!("unresolved type_ref(AliasDoc) = {alias_reexport_aliasdoc_unresolved}"),
        ),
        (
            "Vec<T> subscript through self field  self.segments[0].touch() -> SelfSegment.touch",
            vec_self_field_subscript_touch >= 1,
            format!("resolved-to-SelfSegment edges = {vec_self_field_subscript_touch}"),
        ),
        (
            "self field method (no subscript)  self.name.mark() -> Namer.mark",
            self_field_method_mark >= 1,
            format!("resolved-to-Namer edges = {self_field_method_mark}"),
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
