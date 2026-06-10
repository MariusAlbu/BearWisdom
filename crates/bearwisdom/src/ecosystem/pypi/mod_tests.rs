use std::sync::Arc;

use super::symbol_index::scan_python_header;
use super::*;

#[test]
fn ecosystem_identity() {
    let p = PypiEcosystem;
    assert_eq!(p.id(), ID);
    assert_eq!(Ecosystem::kind(&p), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&p), &["python"]);
}

#[test]
fn legacy_locator_tag_is_python() {
    assert_eq!(ExternalSourceLocator::ecosystem(&PypiEcosystem), "python");
}

#[test]
fn python_name_normalization_strips_extras_and_versions() {
    assert_eq!(normalize_python_dep_name("fastapi"), "fastapi");
    assert_eq!(
        normalize_python_dep_name("fastapi[standard]<1.0.0,>=0.114.2"),
        "fastapi"
    );
    assert_eq!(
        normalize_python_dep_name("pydantic-settings>=2.2.1"),
        "pydantic_settings"
    );
    assert_eq!(normalize_python_dep_name("SQLAlchemy>=2.0"), "sqlalchemy");
    assert_eq!(
        normalize_python_dep_name("psycopg[binary]<4.0.0,>=3.1.13"),
        "psycopg"
    );
}

#[test]
fn python_name_normalization_handles_environment_markers() {
    assert_eq!(
        normalize_python_dep_name("urllib3<2;python_version<'3.10'"),
        "urllib3"
    );
    assert_eq!(
        normalize_python_dep_name("some-pkg @ git+https://github.com/x/y"),
        "some_pkg"
    );
}

#[test]
fn pyproject_pep621_array() {
    let toml = r#"
[project]
name = "test"
dependencies = [
    "fastapi>=0.100",
    "pydantic>=2",
    "sqlalchemy",
]
"#;
    let deps = parse_pyproject_deps(toml);
    assert!(deps.contains(&"fastapi".to_string()));
    assert!(deps.contains(&"pydantic".to_string()));
    assert!(deps.contains(&"sqlalchemy".to_string()));
}

#[test]
fn pyproject_poetry_format() {
    let toml = r#"
[tool.poetry.dependencies]
python = "^3.10"
django = "^4.2"
celery = { extras = ["redis"], version = "^5.3" }
"#;
    let deps = parse_pyproject_deps(toml);
    assert!(deps.contains(&"django".to_string()));
    assert!(deps.contains(&"celery".to_string()));
    assert!(!deps.contains(&"python".to_string()));
}

#[test]
fn requirements_txt_skips_comments_and_urls() {
    let content = "# comment\nrequests==2.28.0\n-r other.txt\nhttp://example.com/pkg.tar.gz\ngit+https://github.com/x/y.git\npandas>=1.5\n";
    let deps = parse_requirements_txt(content);
    assert_eq!(deps, vec!["requests", "pandas"]);
}

#[test]
fn pipfile_parses_packages_section() {
    let content = r#"
[packages]
requests = "*"
pandas = {version = ">=1.5"}

[dev-packages]
pytest = "*"
"#;
    let deps = parse_pipfile_deps(content);
    assert!(deps.contains(&"requests".to_string()));
    assert!(deps.contains(&"pandas".to_string()));
    assert!(deps.contains(&"pytest".to_string()));
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

// -----------------------------------------------------------------
// Header-only Python scanner — demand-driven pipeline entry
// -----------------------------------------------------------------

#[test]
fn scan_captures_class_and_function() {
    let src = "class Client:\n    pass\n\ndef helper():\n    pass\n";
    let names = scan_python_header(src);
    assert!(names.contains(&"Client".to_string()), "{names:?}");
    assert!(names.contains(&"helper".to_string()), "{names:?}");
}

#[test]
fn scan_captures_async_function() {
    let src = "async def fetch(url):\n    pass\n";
    let names = scan_python_header(src);
    assert!(names.contains(&"fetch".to_string()), "{names:?}");
}

#[test]
fn scan_captures_decorated_class_and_function() {
    let src =
        "@dataclass\nclass User:\n    pass\n\n@app.get('/foo')\nasync def handler():\n    pass\n";
    let names = scan_python_header(src);
    assert!(names.contains(&"User".to_string()), "{names:?}");
    assert!(names.contains(&"handler".to_string()), "{names:?}");
}

#[test]
fn scan_captures_module_level_assignments() {
    let src = "VERSION = '1.0'\nMAX_RETRIES: int = 3\n";
    let names = scan_python_header(src);
    assert!(names.contains(&"VERSION".to_string()), "{names:?}");
    assert!(names.contains(&"MAX_RETRIES".to_string()), "{names:?}");
}

#[test]
fn scan_ignores_class_and_function_body_decls() {
    // Names declared inside a class body (methods) or function body
    // (nested defs / locals) must not surface — scanner is header-only.
    let src = r#"
class Outer:
    def hidden_method(self): pass
    INNER = 1

def outer_fn():
    def hidden_inner(): pass
    HIDDEN_LOCAL = 2
"#;
    let names = scan_python_header(src);
    assert!(names.contains(&"Outer".to_string()));
    assert!(names.contains(&"outer_fn".to_string()));
    assert!(
        !names.contains(&"hidden_method".to_string()),
        "leaked: {names:?}"
    );
    assert!(!names.contains(&"INNER".to_string()), "leaked: {names:?}");
    assert!(
        !names.contains(&"hidden_inner".to_string()),
        "leaked: {names:?}"
    );
    assert!(
        !names.contains(&"HIDDEN_LOCAL".to_string()),
        "leaked: {names:?}"
    );
}

#[test]
fn scan_handles_empty_file() {
    assert!(scan_python_header("").is_empty());
}

#[test]
fn build_index_returns_empty_for_no_deps() {
    let idx = build_python_symbol_index(&[]);
    assert!(idx.is_empty());
}
