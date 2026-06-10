use std::path::PathBuf;
use std::sync::Arc;

use super::discovery::{discover_cargo_roots, parse_cargo_lock, split_crate_dir_name};
use super::manifest::parse_cargo_path_dependencies;
use super::reachability::{extract_rust_mod_decls, resolve_rust_mod_path};
use super::symbol_index::scan_rust_header;
use super::*;

#[test]
fn ecosystem_identity() {
    let c = CargoEcosystem;
    assert_eq!(c.id(), ID);
    assert_eq!(Ecosystem::kind(&c), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&c), &["rust"]);
}

#[test]
fn legacy_locator_tag_is_rust() {
    assert_eq!(ExternalSourceLocator::ecosystem(&CargoEcosystem), "rust");
}

// --- Cargo.lock parser ---

#[test]
fn parse_cargo_lock_registry_only() {
    let lock = concat!(
        "version = 3\n\n",
        "[[package]]\n",
        "name = \"anyhow\"\n",
        "version = \"1.0.82\"\n",
        "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        "checksum = \"abc\"\n\n",
        "[[package]]\n",
        "name = \"workspace-crate\"\n",
        "version = \"0.1.0\"\n\n",
        "[[package]]\n",
        "name = \"tokio\"\n",
        "version = \"1.38.0\"\n",
        "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        "checksum = \"def\"\n\n",
        "[[package]]\n",
        "name = \"git-dep\"\n",
        "version = \"0.5.0\"\n",
        "source = \"git+https://github.com/example/crate.git#abc\"\n",
    );
    let entries = parse_cargo_lock(lock);
    assert_eq!(entries.len(), 2);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"anyhow"));
    assert!(names.contains(&"tokio"));
    assert!(!names.contains(&"workspace-crate"));
    assert!(!names.contains(&"git-dep"));
}

#[test]
fn split_crate_dir_name_handles_hyphenated_names() {
    assert_eq!(
        split_crate_dir_name("tokio-1.38.0"),
        Some(("tokio".into(), "1.38.0".into()))
    );
    assert_eq!(
        split_crate_dir_name("proc-macro2-1.0.91"),
        Some(("proc-macro2".into(), "1.0.91".into()))
    );
    assert_eq!(
        split_crate_dir_name("tokio-util-0.7.9"),
        Some(("tokio-util".into(), "0.7.9".into()))
    );
    assert_eq!(split_crate_dir_name("no-version"), None);
}

// --- path deps parser (migrated from manifest/cargo.rs tests) ---

#[test]
fn path_deps_inline_table_single_line() {
    let toml = r#"
[dependencies]
serde = "1"
core = { path = "../core" }
tokio = { version = "1", features = ["full"] }
"#;
    let paths = parse_cargo_path_dependencies(toml);
    assert_eq!(paths, vec!["core"]);
}

#[test]
fn path_deps_multi_line_inline_table() {
    let toml = r#"
[dependencies]
shared = {
    path = "../shared",
    version = "0.1"
}
remote = { version = "1" }
"#;
    let paths = parse_cargo_path_dependencies(toml);
    assert_eq!(paths, vec!["shared"]);
}

#[test]
fn path_deps_across_multiple_dep_sections() {
    let toml = r#"
[dependencies]
core = { path = "../core" }

[dev-dependencies]
testutil = { path = "../testutil" }

[build-dependencies]
builder = { path = "../builder" }
"#;
    let paths = parse_cargo_path_dependencies(toml);
    assert!(paths.contains(&"core".to_string()));
    assert!(paths.contains(&"testutil".to_string()));
    assert!(paths.contains(&"builder".to_string()));
}

#[test]
fn path_deps_ignores_registry_entries() {
    let toml = r#"
[dependencies]
serde = "1"
tokio = { version = "1" }
anyhow = "1.0"
"#;
    let paths = parse_cargo_path_dependencies(toml);
    assert!(paths.is_empty());
}

// --- discovery integration (migrated from externals/rust_lang.rs tests) ---

#[test]
fn discover_cargo_roots_uses_lockfile() {
    let tmp = std::env::temp_dir().join("bw-test-cargo-lock");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    let lock = concat!(
        "version = 3\n\n",
        "[[package]]\n",
        "name = \"serde\"\n",
        "version = \"1.0.200\"\n",
        "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        "checksum = \"abc\"\n",
    );
    std::fs::write(tmp.join("Cargo.lock"), lock).unwrap();

    let fake_home = tmp.join("fake_cargo_home");
    let serde_src = fake_home
        .join("registry")
        .join("src")
        .join("index-abc")
        .join("serde-1.0.200")
        .join("src");
    std::fs::create_dir_all(&serde_src).unwrap();
    std::fs::write(serde_src.join("lib.rs"), "pub trait Serialize {}").unwrap();

    std::env::set_var("CARGO_HOME", fake_home.to_str().unwrap());
    let roots = discover_cargo_roots(&tmp);
    std::env::remove_var("CARGO_HOME");

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "serde");
    assert_eq!(roots[0].version, "1.0.200");

    let walked = walk_cargo_root(&roots[0]);
    assert_eq!(walked.len(), 1);
    assert!(walked[0].relative_path.starts_with("ext:rust:serde/"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn discover_cargo_roots_empty_without_cargo_toml() {
    let tmp = std::env::temp_dir().join("bw-test-cargo-no-toml");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let roots = discover_cargo_roots(&tmp);
    assert!(roots.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

// -----------------------------------------------------------------
// R3 — reachability-based crate entry resolution
// -----------------------------------------------------------------

fn mkdep(root: PathBuf, name: &str, version: &str) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: name.to_string(),
        version: version.to_string(),
        root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn extract_mod_decls_matches_pub_and_bare() {
    let src = r#"
pub mod a;
mod b;
pub(crate) mod c;
pub(super) mod d;
mod e; // inline comment
use foo::bar;
pub mod inline { pub fn f() {} }  // inline body — has no ;
mod ok_end;
"#;
    let decls = extract_rust_mod_decls(src);
    assert!(decls.contains(&"a".to_string()));
    assert!(decls.contains(&"b".to_string()));
    assert!(decls.contains(&"c".to_string()));
    assert!(decls.contains(&"d".to_string()));
    assert!(decls.contains(&"e".to_string()));
    assert!(decls.contains(&"ok_end".to_string()));
    assert!(!decls.contains(&"inline".to_string()));
}

#[test]
fn resolve_crate_entry_follows_mod_tree() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("serde-1.0.200");
    let src = root.join("src");
    std::fs::create_dir_all(src.join("de")).unwrap();
    std::fs::write(src.join("lib.rs"), "pub mod ser;\nmod de;\n").unwrap();
    std::fs::write(src.join("ser.rs"), "pub trait Serialize {}\n").unwrap();
    std::fs::write(src.join("de").join("mod.rs"), "pub mod inner;\n").unwrap();
    std::fs::write(src.join("de").join("inner.rs"), "pub struct Inner;\n").unwrap();

    let dep = mkdep(root.clone(), "serde", "1.0.200");
    let files = CargoEcosystem.resolve_import(&dep, "serde", &["Serialize"]);
    assert_eq!(files.len(), 4, "got: {:?}", files);
    let paths: std::collections::HashSet<_> =
        files.iter().map(|f| f.absolute_path.clone()).collect();
    assert!(paths.contains(&src.join("lib.rs")));
    assert!(paths.contains(&src.join("ser.rs")));
    assert!(paths.contains(&src.join("de").join("mod.rs")));
    assert!(paths.contains(&src.join("de").join("inner.rs")));
    for f in &files {
        assert!(f.relative_path.starts_with("ext:rust:serde/"));
        assert_eq!(f.language, "rust");
    }
}

#[test]
fn resolve_crate_entry_falls_back_to_main_rs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("bin-only-0.1.0");
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("main.rs"), "fn main() {}\n").unwrap();

    let dep = mkdep(root.clone(), "bin-only", "0.1.0");
    let files = CargoEcosystem.resolve_import(&dep, "bin-only", &[]);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].absolute_path, src.join("main.rs"));
}

#[test]
fn resolve_crate_entry_honors_lib_path_in_manifest() {
    // Tree-sitter and other C-with-Rust-bindings crates declare
    // their entry via `[lib] path = "binding_rust/lib.rs"`. The
    // walker must read that field; without it the crate's binding
    // surface is invisible.
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("tree-sitter-0.25.10");
    let bindings = root.join("binding_rust");
    std::fs::create_dir_all(&bindings).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"tree-sitter\"\n[lib]\nname = \"tree_sitter\"\npath = \"binding_rust/lib.rs\"\n",
    ).unwrap();
    std::fs::write(
        bindings.join("lib.rs"),
        "pub struct Node;\nimpl Node { pub fn prev_sibling(&self) -> Option<Node> { None } }\n",
    )
    .unwrap();

    let dep = mkdep(root.clone(), "tree-sitter", "0.25.10");
    let files = CargoEcosystem.resolve_import(&dep, "tree-sitter", &[]);
    assert_eq!(files.len(), 1, "got: {:?}", files);
    assert_eq!(files[0].absolute_path, bindings.join("lib.rs"));
}

#[test]
fn resolve_crate_entry_empty_without_src_entry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("no-entry-0.1.0");
    std::fs::create_dir_all(&root).unwrap();

    let dep = mkdep(root, "no-entry", "0.1.0");
    assert!(CargoEcosystem
        .resolve_import(&dep, "no-entry", &[])
        .is_empty());
}

#[test]
fn resolve_rust_mod_path_handles_both_layouts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    let lib = src.join("lib.rs");
    std::fs::write(&lib, "").unwrap();

    // sibling .rs layout
    let a = src.join("a.rs");
    std::fs::write(&a, "").unwrap();
    assert_eq!(resolve_rust_mod_path(&lib, "a"), Some(a));

    // directory/mod.rs layout
    let sub_mod = src.join("sub").join("mod.rs");
    std::fs::write(&sub_mod, "").unwrap();
    assert_eq!(resolve_rust_mod_path(&lib, "sub"), Some(sub_mod));

    // missing module
    assert_eq!(resolve_rust_mod_path(&lib, "missing"), None);
}

#[test]
fn rust_header_scanner_captures_top_level_items() {
    let src = r#"
pub struct Foo {
    x: i32,
}

pub enum Status { Ok, Err }

pub trait Service {
    fn call(&self) -> Result<(), ()>;
}

pub fn top_level_fn() -> i32 { 0 }

pub const MAX: usize = 10;

pub static NAME: &str = "x";

pub type Alias = Foo;

macro_rules! my_macro { () => {}; }

impl Foo {
    pub fn new() -> Self { Foo { x: 0 } }
    pub fn helper(&self) {}
}
"#;
    let names = scan_rust_header(src);
    assert!(names.contains(&"Foo".to_string()), "{names:?}");
    assert!(names.contains(&"Status".to_string()), "{names:?}");
    assert!(names.contains(&"Service".to_string()), "{names:?}");
    assert!(names.contains(&"top_level_fn".to_string()), "{names:?}");
    assert!(names.contains(&"MAX".to_string()), "{names:?}");
    assert!(names.contains(&"NAME".to_string()), "{names:?}");
    assert!(names.contains(&"Alias".to_string()), "{names:?}");
    assert!(names.contains(&"new".to_string()), "{names:?}");
    assert!(names.contains(&"Foo::new".to_string()), "{names:?}");
    assert!(names.contains(&"Foo::helper".to_string()), "{names:?}");
    // Trait body methods — added so the SymbolLocationIndex can locate
    // trait files by method name (motivator: axum's IntoResponse).
    assert!(names.contains(&"call".to_string()), "{names:?}");
    assert!(names.contains(&"Service::call".to_string()), "{names:?}");
}

#[test]
fn rust_build_symbol_index_empty_returns_empty() {
    assert!(build_cargo_symbol_index(&[]).is_empty());
}
