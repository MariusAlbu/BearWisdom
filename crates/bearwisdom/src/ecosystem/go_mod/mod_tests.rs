use std::path::PathBuf;
use std::sync::Arc;

use super::discovery::{collect_module_imports, escape_module_path};
use super::reachability::resolve_go_requested_packages;
use super::symbol_index::scan_go_header;
use super::*;

#[test]
fn ecosystem_identity() {
    let g = GoModEcosystem;
    assert_eq!(g.id(), ID);
    assert_eq!(Ecosystem::kind(&g), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&g), &["go"]);
}

#[test]
fn legacy_locator_tag_is_go() {
    assert_eq!(ExternalSourceLocator::ecosystem(&GoModEcosystem), "go");
}

#[test]
fn escape_preserves_lowercase_paths() {
    assert_eq!(
        escape_module_path("github.com/gin-gonic/gin"),
        "github.com/gin-gonic/gin"
    );
}

#[test]
fn escape_handles_uppercase_segments() {
    assert_eq!(
        escape_module_path("github.com/Microsoft/go-winio"),
        "github.com/!microsoft/go-winio"
    );
    assert_eq!(
        escape_module_path("github.com/AlecAivazis/survey"),
        "github.com/!alec!aivazis/survey"
    );
}

#[test]
fn discover_returns_empty_without_go_mod() {
    let tmp = std::env::temp_dir().join("bw-test-gomod-empty");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let result = discover_go_externals(&tmp);
    assert!(result.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn parse_go_mod_basic() {
    let content = r#"module foo.example/bar

go 1.21

require (
    github.com/gin-gonic/gin v1.9.1
    github.com/stretchr/testify v1.9.0 // indirect
)

require github.com/other/pkg v1.0.0
"#;
    let data = parse_go_mod(content);
    assert_eq!(data.module_path.as_deref(), Some("foo.example/bar"));
    assert_eq!(data.require_deps.len(), 3);
    assert!(data
        .require_deps
        .iter()
        .any(|d| d.path == "github.com/gin-gonic/gin" && !d.indirect));
    assert!(data
        .require_deps
        .iter()
        .any(|d| d.path == "github.com/stretchr/testify" && d.indirect));
    assert!(data
        .require_deps
        .iter()
        .any(|d| d.path == "github.com/other/pkg"));
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

// -----------------------------------------------------------------
// Header-only scanner — tree-sitter parse without body descent
// -----------------------------------------------------------------

#[test]
fn scan_captures_top_level_function() {
    let src = r#"
package sqlite

func Open(path string) (*DB, error) {
    // body we never walk
    return nil, nil
}
"#;
    let names = scan_go_header(src);
    assert!(names.contains(&"Open".to_string()), "names: {names:?}");
}

#[test]
fn scan_captures_method_with_pointer_receiver() {
    let src = r#"
package sqlite

type DB struct{}

func (d *DB) Query(q string) (*Rows, error) {
    return nil, nil
}
"#;
    let names = scan_go_header(src);
    assert!(
        names.contains(&"DB".to_string()),
        "types missing: {names:?}"
    );
    assert!(
        names.contains(&"DB.Query".to_string()),
        "method missing: {names:?}"
    );
}

#[test]
fn scan_captures_method_with_value_receiver() {
    let src = r#"
package sqlite

type Query struct{}

func (q Query) String() string { return "" }
"#;
    let names = scan_go_header(src);
    assert!(names.contains(&"Query.String".to_string()), "{names:?}");
}

#[test]
fn scan_captures_type_declarations() {
    let src = r#"
package foo

type Client struct { addr string }
type Handler interface { Handle() }
type Name = string
"#;
    let names = scan_go_header(src);
    assert!(names.contains(&"Client".to_string()), "{names:?}");
    assert!(names.contains(&"Handler".to_string()), "{names:?}");
    assert!(names.contains(&"Name".to_string()), "{names:?}");
}

#[test]
fn scan_captures_top_level_vars_and_consts() {
    let src = r#"
package foo

var DefaultTimeout = 30
const MaxRetries = 3

var (
    LogLevel = "info"
    Verbose  bool
)
"#;
    let names = scan_go_header(src);
    for expected in ["DefaultTimeout", "MaxRetries", "LogLevel", "Verbose"] {
        assert!(
            names.contains(&expected.to_string()),
            "missing {expected}: {names:?}"
        );
    }
}

#[test]
fn scan_ignores_function_body_contents() {
    // Identifiers inside function bodies must not leak into the top-level
    // name set — the scanner is *header-only*.
    let src = r#"
package foo

func Outer() {
    var shouldNotAppear = 1
    type AlsoHidden struct{}
    _ = shouldNotAppear
}
"#;
    let names = scan_go_header(src);
    assert_eq!(names, vec!["Outer".to_string()]);
}

#[test]
fn scan_handles_generic_method_receiver() {
    // `func (c *Cache[K, V]) Get(k K) V` — receiver type unwraps to `Cache`.
    let src = r#"
package foo

type Cache[K comparable, V any] struct{}

func (c *Cache[K, V]) Get(k K) V { var zero V; return zero }
"#;
    let names = scan_go_header(src);
    assert!(names.contains(&"Cache".to_string()), "type: {names:?}");
    assert!(
        names.contains(&"Cache.Get".to_string()),
        "method: {names:?}"
    );
}

#[test]
fn scan_returns_empty_on_unparseable_source() {
    // Tree-sitter returns an error tree rather than None, but we should
    // still surface whatever valid top-level decls it finds.
    let names = scan_go_header("not valid go");
    assert!(names.is_empty());
}

#[test]
fn build_index_returns_empty_for_no_deps() {
    let idx = build_go_symbol_index(&[]);
    assert!(idx.is_empty());
}

#[test]
fn build_index_populates_from_on_disk_files() {
    use std::io::Write;
    let tmp = std::env::temp_dir().join("bw-test-gomod-symindex");
    let _ = std::fs::remove_dir_all(&tmp);
    let pkg_dir = tmp.join("lib");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    let sqlite_go = pkg_dir.join("sqlite.go");
    let mut f = std::fs::File::create(&sqlite_go).unwrap();
    writeln!(
        f,
        "package sqlite\n\ntype DB struct {{}}\nfunc Open() *DB {{ return nil }}\nfunc (d *DB) Query() error {{ return nil }}"
    ).unwrap();

    let dep = ExternalDepRoot {
        module_path: "modernc.org/sqlite".to_string(),
        version: "v0".to_string(),
        root: tmp.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: vec!["modernc.org/sqlite/lib".to_string()],
    };
    let idx = build_go_symbol_index(std::slice::from_ref(&dep));

    assert_eq!(
        idx.locate("modernc.org/sqlite", "Open"),
        Some(sqlite_go.as_path())
    );
    assert_eq!(
        idx.locate("modernc.org/sqlite", "DB"),
        Some(sqlite_go.as_path())
    );
    assert_eq!(
        idx.locate("modernc.org/sqlite", "DB.Query"),
        Some(sqlite_go.as_path())
    );
    assert_eq!(idx.locate("modernc.org/sqlite", "Nonexistent"), None);

    let _ = std::fs::remove_dir_all(&tmp);
}

// -----------------------------------------------------------------
// R3 — user-import-narrowed sub-package walking
// -----------------------------------------------------------------

fn mkdep(root: PathBuf, module: &str, requested: Vec<String>) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module.to_string(),
        version: "v1.0.0".to_string(),
        root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: requested,
    }
}

#[test]
fn collect_module_imports_returns_matching_paths() {
    let mut set = std::collections::HashSet::new();
    set.insert("github.com/gin-gonic/gin".to_string());
    set.insert("github.com/gin-gonic/gin/binding".to_string());
    set.insert("github.com/gin-gonic/gin/render".to_string());
    set.insert("github.com/other/pkg".to_string());
    set.insert("github.com/gin-gonic/gink".to_string()); // prefix collision — must NOT match
    let got = collect_module_imports("github.com/gin-gonic/gin", &set);
    assert_eq!(
        got,
        vec![
            "github.com/gin-gonic/gin".to_string(),
            "github.com/gin-gonic/gin/binding".to_string(),
            "github.com/gin-gonic/gin/render".to_string(),
        ]
    );
}

#[test]
fn resolve_walks_only_requested_sub_packages() {
    let tmp = std::env::temp_dir().join("bw-test-go-r3-narrow");
    let _ = std::fs::remove_dir_all(&tmp);
    let root = tmp.join("gin@v1.0.0");
    let binding = root.join("binding");
    let internal = root.join("internal");
    std::fs::create_dir_all(&binding).unwrap();
    std::fs::create_dir_all(&internal).unwrap();
    std::fs::write(root.join("gin.go"), "package gin\n").unwrap();
    std::fs::write(binding.join("binding.go"), "package binding\n").unwrap();
    std::fs::write(internal.join("internal.go"), "package internal\n").unwrap();

    let dep = mkdep(
        root.clone(),
        "github.com/gin-gonic/gin",
        vec!["github.com/gin-gonic/gin/binding".to_string()],
    );
    let files = resolve_go_requested_packages(&dep);
    let paths: std::collections::HashSet<_> =
        files.iter().map(|f| f.absolute_path.clone()).collect();
    assert!(
        paths.contains(&binding.join("binding.go")),
        "expected binding.go to be walked, got: {paths:?}"
    );
    assert!(
        !paths.contains(&root.join("gin.go")),
        "root package should NOT be walked when not requested: {paths:?}"
    );
    assert!(
        !paths.contains(&internal.join("internal.go")),
        "unrequested sibling should NOT be walked: {paths:?}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_follows_within_module_transitive_imports() {
    let tmp = std::env::temp_dir().join("bw-test-go-r3-transitive");
    let _ = std::fs::remove_dir_all(&tmp);
    let root = tmp.join("myMod@v1.0.0");
    let a = root.join("a");
    let b = root.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    // a/a.go imports b, so walking a should pull in b.
    std::fs::write(
        a.join("a.go"),
        "package a\nimport \"my.example/myMod/b\"\nvar _ = b.X\n",
    )
    .unwrap();
    std::fs::write(b.join("b.go"), "package b\nvar X = 1\n").unwrap();

    let dep = mkdep(
        root.clone(),
        "my.example/myMod",
        vec!["my.example/myMod/a".to_string()],
    );
    let files = resolve_go_requested_packages(&dep);
    let paths: std::collections::HashSet<_> =
        files.iter().map(|f| f.absolute_path.clone()).collect();
    assert!(paths.contains(&a.join("a.go")));
    assert!(
        paths.contains(&b.join("b.go")),
        "transitive within-module import should be followed: {paths:?}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn resolve_falls_back_to_walk_root_when_no_requested_imports() {
    let tmp = std::env::temp_dir().join("bw-test-go-r3-fallback");
    let _ = std::fs::remove_dir_all(&tmp);
    let root = tmp.join("mod@v1.0.0");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.go"), "package mod\n").unwrap();

    let dep = mkdep(root.clone(), "my.example/mod", Vec::new());
    let files = resolve_go_requested_packages(&dep);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].absolute_path, root.join("a.go"));
    let _ = std::fs::remove_dir_all(&tmp);
}
