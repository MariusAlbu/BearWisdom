use super::super::contract::SymbolLookup;
use super::super::{compilation::Compilation, file_lookup::FileLookup, testkit};
use super::*;
use crate::{
    type_checker::core::types::TypeArena,
    types::{EdgeKind, SymbolKind},
};
use std::sync::Arc;

#[test]
fn configured_custom_root_and_out_of_line_rename_bind_the_constructor_cascade() {
    check_project(&[
        ("Cargo.toml", "[package]\nname='sample'\n[lib]\nname='public_api'\npath='custom/root.rs'"),
        ("custom/root.rs", "mod real; pub use real::RealDoc as AliasDoc;"),
        ("custom/real.rs", "pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} }"),
        ("src/lib.rs", "pub struct AliasDoc; impl AliasDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} }"),
        ("benches/run.rs", "use public_api::AliasDoc; fn f() { AliasDoc::new().touch(); }"),
    ], "benches/run.rs", Some("custom/real.rs"));
}

const DOC: &str =
    "pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} }";

#[test]
fn unknown_bare_factories_cannot_seed_same_file_namesake_cascades() {
    for (generics, arguments) in [("", ""), ("<T>", "::<u32>")] {
        let provider =
            format!("mod unrelated {{ pub fn make{generics}() -> super::b::Doc {{ loop {{}} }} }}");
        let caller = format!(
            "fn f() {{ make{arguments}().touch(); let p = make{arguments}(); p.touch(); }}"
        );
        check_locals(&format!("{provider} {caller}"), &[]);
        check_locals(
            &format!("{provider} use unrelated::make; {caller}"),
            &[
                "unrelated.make",
                "b.Doc.touch",
                "unrelated.make",
                "b.Doc.touch",
            ],
        );
    }
}

#[test]
fn configured_missing_private_conditional_and_conflicting_exports_never_borrow_namesakes() {
    for root in [
        "",
        "mod real;",
        "#[cfg(feature = \"opt\")] mod real; pub use real::RealDoc as AliasDoc;",
        "pub mod ambiguous; pub use ambiguous::RealDoc as AliasDoc;",
        "#[path=\"missing.rs\"] mod real; pub use real::RealDoc as AliasDoc;",
        "#[path=\"real.rs\"] mod a; #[path=\"real.rs\"] mod b; pub use a::RealDoc as AliasDoc;",
    ] {
        check_project(
            &[
                ("Cargo.toml", "[package]\nname='sample'"),
                ("src/lib.rs", root),
                ("src/real.rs", DOC),
                ("src/ambiguous.rs", DOC),
                ("src/ambiguous/mod.rs", DOC),
                (
                    "benches/run.rs",
                    "use sample::AliasDoc; fn f() { AliasDoc::new().touch(); }",
                ),
            ],
            "benches/run.rs",
            None,
        );
    }
}

#[test]
fn exact_module_paths_include_directory_entry_inline_ancestry_and_path_attributes() {
    for (root, file) in [
        ("pub mod api; pub use api::RealDoc as AliasDoc;", "src/api/mod.rs"),
        ("pub mod inline { pub mod api; } pub use inline::api::RealDoc as AliasDoc;", "src/inline/api.rs"),
        ("#[path=\"layout/real.rs\"] mod api; pub use api::RealDoc as AliasDoc;", "src/layout/real.rs"),
        ("#[path=\"layout\"] mod inline { #[path=\"real.rs\"] pub mod api; } pub use inline::api::RealDoc as AliasDoc;", "src/layout/real.rs"),
    ] {
        check_project(&[("Cargo.toml", "[package]\nname='sample'"), ("src/lib.rs", root), (file, DOC),
            ("benches/run.rs", "use sample::AliasDoc; fn f() { AliasDoc::new().touch(); }")], "benches/run.rs", Some(file));
    }
}

#[test]
fn cross_file_parent_and_crate_roots_are_bound_as_numeric_module_relationships() {
    for import in ["super::RealDoc", "crate::api::RealDoc"] {
        check_project(
            &[
                ("Cargo.toml", "[package]\nname='sample'"),
                ("src/lib.rs", "pub mod api;"),
                ("src/api.rs", &format!("{DOC} pub mod child;")),
                (
                    "src/api/child.rs",
                    &format!("use {import} as AliasDoc; fn f() {{ AliasDoc::new().touch(); }}"),
                ),
                ("src/decoy.rs", DOC),
            ],
            "src/api/child.rs",
            Some("src/api.rs"),
        );
    }
}

#[test]
fn dependency_renames_are_consumer_scoped_and_honor_custom_library_paths() {
    let files = [
        (
            "Cargo.toml",
            "[workspace]\nmembers=['one','two','left','right']",
        ),
        (
            "left/Cargo.toml",
            "[package]\nname='left'\n[lib]\npath='custom/root.rs'\nname='left_api'",
        ),
        ("left/custom/root.rs", DOC),
        ("right/Cargo.toml", "[package]\nname='right'"),
        ("right/src/lib.rs", DOC),
        (
            "one/Cargo.toml",
            "[package]\nname='one'\n[dependencies]\napi={package='left',path='../left'}",
        ),
        (
            "one/src/lib.rs",
            "use api::RealDoc as AliasDoc; fn f() { AliasDoc::new().touch(); }",
        ),
        (
            "two/Cargo.toml",
            "[package]\nname='two'\n[dependencies]\napi={package='right',path='../right'}",
        ),
        (
            "two/src/lib.rs",
            "use api::RealDoc as AliasDoc; fn f() { AliasDoc::new().touch(); }",
        ),
    ];
    check_project(&files, "one/src/lib.rs", Some("left/custom/root.rs"));
    check_project(&files, "two/src/lib.rs", Some("right/src/lib.rs"));
}

#[test]
fn dependency_target_scopes_and_unevaluated_conditions_are_not_global_imports() {
    for (section, source_file, allowed, extra) in [
        ("dev-dependencies", "src/lib.rs", false, ""),
        ("dev-dependencies", "benches/run.rs", true, ""),
        ("build-dependencies", "src/lib.rs", false, ""),
        ("build-dependencies", "build.rs", true, ""),
        ("dependencies", "src/lib.rs", false, ",optional=true"),
    ] {
        let mut files = vec![
            ("Cargo.toml", format!("[package]\nname='sample'\n[{section}]\napi={{package='provider',path='provider'{extra}}}")),
            ("provider/Cargo.toml", "[package]\nname='provider'".into()), ("provider/src/lib.rs", DOC.into()),
            (source_file, "use api::RealDoc as AliasDoc; fn f() { AliasDoc::new().touch(); }".into()),
        ];
        if source_file != "src/lib.rs" {
            files.push(("src/lib.rs", "".into()));
        }
        check_project(
            &files
                .iter()
                .map(|(path, body)| (*path, body.as_str()))
                .collect::<Vec<_>>(),
            source_file,
            allowed.then_some("provider/src/lib.rs"),
        );
    }
}

#[test]
fn one_physical_file_used_by_multiple_configured_crates_cannot_claim_a_unique_namespace() {
    check_project(&[("Cargo.toml", "[package]\nname='sample'\n[lib]\npath='shared.rs'\n[[bin]]\nname='tool'\npath='shared.rs'"),
        ("shared.rs", DOC), ("benches/run.rs", "use sample::RealDoc as AliasDoc; fn f() { AliasDoc::new().touch(); }")],
        "benches/run.rs", None);
}

fn check_project(files: &[(&str, &str)], source_file: &str, expected_file: Option<&str>) {
    check_project_mode(files, source_file, expected_file, false);
}

#[test]
fn rejected_constructor_cannot_fabricate_a_downstream_local_type() {
    for root in [
        "",
        "mod real;",
        "mod real; pub use real::RealDoc as AliasDoc;",
    ] {
        check_project_mode(&[("Cargo.toml", "[package]\nname='sample'"), ("src/lib.rs", root),
            ("src/real.rs", DOC),
            ("src/decoy.rs", "pub struct AliasDoc; impl AliasDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} }"),
            ("benches/run.rs", "use sample::AliasDoc; fn f() { let p = AliasDoc::new(); p.touch(); }")],
            "benches/run.rs", root.contains("pub use").then_some("src/real.rs"), true);
    }
}

fn check_project_mode(
    files: &[(&str, &str)],
    source_file: &str,
    expected_file: Option<&str>,
    pipeline: bool,
) {
    with_project(files, pipeline, |tree, parsed, ids| {
        if pipeline {
            verify_pipeline(tree, parsed, ids, source_file, expected_file);
        } else {
            verify_project(tree, parsed, ids, source_file, expected_file);
        }
    });
}

fn with_project(
    files: &[(&str, &str)],
    pipeline: bool,
    mut verify: impl FnMut(&Compilation, &[ParsedFile], &SymbolIds),
) {
    let dir = tempfile::tempdir().unwrap();
    let arena = Arc::new(TypeArena::new());
    let mut parsed = Vec::new();
    for &(path, source) in files {
        let absolute = dir.path().join(path);
        std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        std::fs::write(&absolute, source).unwrap();
        if path.ends_with(".rs") {
            parsed.push(
                crate::indexer::parse_file::parse_file_with_arena(
                    &crate::walker::WalkedFile {
                        relative_path: path.into(),
                        absolute_path: absolute,
                        language: "rust",
                    },
                    &crate::languages::default_registry(),
                    &arena,
                )
                .unwrap(),
            );
        }
    }
    let context = crate::indexer::project_context::build_project_context(dir.path());
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        &parsed,
        "internal",
        Some(&arena),
    )
    .unwrap();
    let mut tree =
        Compilation::build_with_context(&parsed, &ids, arena, Some(&context), &Default::default());
    if pipeline {
        super::super::inference_prelude::run(
            &mut tree,
            &parsed,
            &ids,
            &super::super::file_context::build_profiles(),
        );
    }
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    let snapshot: String = db
        .conn()
        .query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    restored.restore_snapshot(&snapshot);
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    for tree in [&tree, &cold] {
        verify(tree, &parsed, &ids);
    }
}

const LOCAL_TYPES: &str = "mod a { pub struct Doc; impl Doc { pub fn new() -> Self { Self } pub fn touch(&self) {} pub fn convert(&self) -> super::b::Doc { loop {} } } }
    mod b { pub struct Doc; impl Doc { pub fn new() -> Self { Self } pub fn touch(&self) {} } }";

#[test]
fn composite_source_types_keep_generic_field_and_return_heads_bound() {
    check_locals(
        "mod api { pub struct Holder<T> { pub inner: T } }
        fn f(p: api::Holder<b::Doc>) { p.inner.touch(); }",
        &["b.Doc.touch"],
    );
    check_locals(
        "mod api { pub struct Holder<T> { pub inner: T } }
        fn make() -> api::Holder<b::Doc> { loop {} } fn f() { let p = make(); p.inner.touch(); }",
        &["make", "b.Doc.touch"],
    );
    check_locals(
        "mod api { pub struct Holder<T> { pub inner: T } }
        fn f(p: api::Holder<missing::Doc>) { p.inner.touch(); }",
        &[],
    );
}

#[test]
fn composite_aliases_references_and_namesake_fields_preserve_argument_ids() {
    check_locals(
        "mod api { pub struct Holder<T> { pub inner: T } }
        type Wrapped<T> = api::Holder<T>;
        fn f(p: &Wrapped<b::Doc>) { p.inner.touch(); }
        fn g(p: Wrapped<a::Doc>) { p.inner.touch(); }",
        &["b.Doc.touch", "a.Doc.touch"],
    );
    check_locals(
        "mod x { pub struct Holder<T> { pub inner: T } }
        mod y { pub struct Holder<T> { pub inner: T } }
        struct Outer { field: y::Holder<b::Doc> }
        fn f(p: Outer) { p.field.inner.touch(); }",
        &["b.Doc.touch"],
    );
    check_locals(
        "mod api { pub struct Holder<T> { pub inner: T } }
        type Wrapped<T> = api::Holder<T>;
        fn f(p: Wrapped<missing::Doc>) { p.inner.touch(); }",
        &[],
    );
}

#[test]
fn constructed_values_and_copies_do_not_merge_namesakes_or_future_shadowing() {
    check_locals(
        "fn f() { let p = a::Doc {}; let q = p; let p = b::Doc {}; q.touch(); p.touch(); }",
        &["a.Doc.touch", "b.Doc.touch"],
    );
    check_locals(
        "fn f() { let p = b::Doc {}; let p = p; p.touch(); }",
        &["b.Doc.touch"],
    );
    check_locals(
        "fn f() { let p = b::Doc {}; let p = missing::Doc {}; let q = p; q.touch(); }",
        &[],
    );
    check_locals(
        "mod api { pub struct Holder<T> { pub inner: T } }
        fn f() { let p = api::Holder::<b::Doc> { inner: b::Doc {} }; let q = p; q.inner.touch(); }",
        &["b.Doc.touch"],
    );
}

#[test]
fn composite_exported_returns_retain_nested_import_ids_after_cold_reload() {
    for root in [
        "pub mod api; pub mod real;",
        "pub mod api; mod real;",
        "pub mod api;",
    ] {
        with_project(
            &[
                ("Cargo.toml", "[package]\nname='sample'"),
                ("src/lib.rs", root),
                ("src/real.rs", DOC),
                ("src/decoy.rs", DOC),
                (
                    "src/api.rs",
                    "pub struct Holder<T> { pub inner: T }
                pub fn make() -> Holder<crate::real::RealDoc> { loop {} }",
                ),
                (
                    "benches/run.rs",
                    "use sample::api::make; fn f() { let p = make(); let q = p; q.inner.touch(); }",
                ),
            ],
            true,
            |tree, parsed, ids| {
                let source = parsed.iter().find(|p| p.path == "benches/run.rs").unwrap();
                let solver = super::super::semantic_model::SemanticModel::production();
                let (edges, _, _, _) = super::super::pipeline::resolve_one_file(
                    source,
                    tree,
                    &super::super::file_context::build_profiles(),
                    &Default::default(),
                    None,
                    &solver,
                    ids,
                    None,
                );
                let actual: Vec<_> = edges
                    .iter()
                    .filter(|e| tree.symbol_by_id(e.1).is_some_and(|s| s.name == "touch"))
                    .map(|e| e.1)
                    .collect();
                let real = parsed.iter().find(|p| p.path == "src/real.rs").unwrap();
                let slot = real.symbols.iter().position(|s| s.name == "touch").unwrap();
                let expected: Vec<_> = root
                    .contains("mod real;")
                    .then(|| ids.row_id(&real.path, slot).unwrap())
                    .into_iter()
                    .collect();
                assert_eq!(actual, expected, "root: {root}");
            },
        );
    }
}

#[test]
fn private_module_paths_allow_internal_siblings_without_exposing_external_namesakes() {
    let root = "mod hidden; pub mod api; pub use hidden::RealDoc as PublicDoc;";
    for (source, body, expected) in [
        (
            "src/api.rs",
            "fn f() { let p = crate::hidden::RealDoc::new(); p.touch(); }",
            Some("src/hidden.rs"),
        ),
        (
            "benches/run.rs",
            "fn f() { let p = sample::PublicDoc::new(); p.touch(); }",
            Some("src/hidden.rs"),
        ),
        (
            "benches/run.rs",
            "fn f() { let p = sample::hidden::RealDoc::new(); p.touch(); }",
            None,
        ),
    ] {
        check_project_mode(
            &[
                ("Cargo.toml", "[package]\nname='sample'"),
                ("src/lib.rs", root),
                ("src/hidden.rs", DOC),
                ("src/decoy.rs", DOC),
                (source, body),
            ],
            source,
            expected,
            true,
        );
    }
}

#[test]
fn private_terminal_items_cannot_be_widened_by_illegal_public_reexports() {
    let hidden = DOC.replace("pub struct", "struct");
    check_project_mode(
        &[
            ("Cargo.toml", "[package]\nname='sample'"),
            (
                "src/lib.rs",
                "mod hidden; pub use hidden::RealDoc as PublicDoc;",
            ),
            ("src/hidden.rs", &hidden),
            ("src/decoy.rs", DOC),
            (
                "benches/run.rs",
                "fn f() { let p = sample::PublicDoc::new(); p.touch(); }",
            ),
        ],
        "benches/run.rs",
        None,
        true,
    );
}

#[test]
fn explicit_module_paths_cannot_retry_missing_members_as_configured_dependencies() {
    for (body, expected) in explicit_path_cases() {
        check_project_mode(
            &[
                (
                    "Cargo.toml",
                    "[package]\nname='sample'\n[dependencies]\napi={path='provider'}",
                ),
                ("provider/Cargo.toml", "[package]\nname='api'"),
                ("provider/src/lib.rs", DOC),
                ("src/lib.rs", body),
                ("src/decoy.rs", DOC),
            ],
            "src/lib.rs",
            expected,
            true,
        );
    }
}

fn explicit_path_cases() -> [(&'static str, Option<&'static str>); 4] {
    [
        (
            "fn f() { let p = api::RealDoc::new(); p.touch(); }",
            Some("provider/src/lib.rs"),
        ),
        (
            "fn f() { let p = self::api::RealDoc::new(); p.touch(); }",
            None,
        ),
        (
            "mod inner { fn f() { let p = super::api::RealDoc::new(); p.touch(); } }",
            None,
        ),
        (
            "use api as local; fn f() { let p = self::local::RealDoc::new(); p.touch(); }",
            Some("provider/src/lib.rs"),
        ),
    ]
}

#[test]
fn named_ancestor_access_scopes_admit_only_their_descendants_fresh_and_cold() {
    for restriction in ["crate::outer", "super", "super::super::outer"] {
        let inner = DOC.replace("pub struct", &format!("pub(in {restriction}) struct"));
        let caller = "fn f() { let p = crate::outer::inner::RealDoc::new(); p.touch(); }";
        with_project(
            &[
                ("Cargo.toml", "[package]\nname='sample'"),
                ("src/lib.rs", "pub mod outer; mod outsider;"),
                ("src/outer.rs", "pub mod inner; mod sibling;"),
                ("src/outer/inner.rs", &inner),
                ("src/outer/sibling.rs", caller),
                ("src/outsider.rs", caller),
                ("src/decoy.rs", DOC),
            ],
            true,
            |tree, parsed, ids| {
                verify_pipeline(
                    tree,
                    parsed,
                    ids,
                    "src/outer/sibling.rs",
                    Some("src/outer/inner.rs"),
                );
                verify_pipeline(tree, parsed, ids, "src/outsider.rs", None);
            },
        );
    }
}

#[test]
fn named_restrictions_cannot_use_aliases_non_ancestors_missing_or_ambiguous_modules() {
    for (restriction, extra, allowed) in restriction_cases() {
        check_locals(
            &restriction_fixture(restriction, extra),
            if allowed {
                &["outer.inner.RealDoc.new", "outer.inner.RealDoc.touch"]
            } else {
                &[]
            },
        );
    }
}

fn restriction_cases() -> [(&'static str, &'static str, bool); 7] {
    [
        ("crate::outer", "", true),
        ("crate::shortcut", "use crate::outer as shortcut;", false),
        ("crate::outsider", "mod outsider {}", false),
        ("crate::missing", "", false),
        ("crate::outer", "mod outer {}", false),
        ("self", "", false),
        ("super::super", "", true),
    ]
}

fn restriction_fixture(restriction: &str, extra: &str) -> String {
    let doc = DOC.replace("pub struct", &format!("pub(in {restriction}) struct"));
    format!(
        "mod outer {{ pub mod inner {{ {doc} }}
        mod sibling {{ fn f() {{ let p = super::inner::RealDoc::new(); p.touch(); }} }} }} {extra}"
    )
}

#[test]
#[ignore = "independent module path/restriction check; requires rustc on PATH"]
fn module_path_and_restriction_fixtures_agree_with_rustc() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("api.rs");
    std::fs::write(&input, DOC).unwrap();
    let provider = dir.path().join("libapi.rmeta");
    let output = std::process::Command::new("rustc")
        .args([
            "--crate-name",
            "api",
            "--crate-type",
            "lib",
            "--edition=2021",
            "--emit=metadata",
        ])
        .arg(&input)
        .arg("-o")
        .arg(&provider)
        .output()
        .expect("rustc required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut cases: Vec<_> = explicit_path_cases()
        .into_iter()
        .map(|(source, expected)| (source.to_owned(), expected.is_some()))
        .collect();
    cases.extend(
        restriction_cases()
            .into_iter()
            .map(|(restriction, extra, allowed)| {
                (restriction_fixture(restriction, extra), allowed)
            }),
    );
    for (source, allowed) in cases {
        let input = dir.path().join("consumer.rs");
        std::fs::write(&input, &source).unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--crate-name",
                "consumer",
                "--crate-type",
                "lib",
                "--edition=2021",
                "--emit=metadata",
                "--cap-lints=allow",
                "--extern",
            ])
            .arg(format!("api={}", provider.display()))
            .arg(&input)
            .arg("-o")
            .arg(dir.path().join("consumer.rmeta"))
            .output()
            .expect("rustc required");
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), allowed, "{source}\n{diagnostics}");
        if !allowed {
            assert!(["E0433", "E0603", "E0742", "E0428"].iter().any(|code| diagnostics.contains(code)),
                "negative must fail for module lookup, privacy, ancestry or duplicate declarations: {diagnostics}");
        }
    }
}

#[test]
fn named_restrictions_preserve_reexport_ceilings_and_composite_return_provenance() {
    let doc = DOC.replace("pub struct", "pub(in crate::outer) struct");
    for reexport in ["pub use", "pub(in crate::outer) use"] {
        let source = format!(
            "mod outer {{ mod inner {{ {doc} }}
            mod api {{ {reexport} super::inner::RealDoc as Alias; }}
            fn f() {{ let p = api::Alias::new(); p.touch(); }} }}"
        );
        check_locals(
            &source,
            if reexport == "pub use" {
                &[]
            } else {
                &["outer.inner.RealDoc.new", "outer.inner.RealDoc.touch"]
            },
        );
    }
    check_locals(
        "mod outer { mod inner {
            pub(in crate::outer) struct Holder<T> { pub value: T }
            pub(super) fn make() -> Holder<crate::b::Doc> { loop {} }
        }
        mod sibling { fn f() { let p = super::inner::make(); p.value.touch(); } }
    }",
        &["outer.inner.make", "b.Doc.touch"],
    );
}

#[test]
fn module_access_distinguishes_siblings_descendants_and_external_requesters_in_one_snapshot() {
    for (visibility, sibling_access) in [
        ("", false),
        ("pub(self)", false),
        ("pub(super)", true),
        ("pub(crate)", true),
        ("pub", true),
    ] {
        let hidden = format!(
            "{} mod child;",
            DOC.replace("pub struct", &format!("{visibility} struct"))
        );
        let caller = "fn f() { let p = crate::hidden::RealDoc::new(); p.touch(); }";
        with_project(
            &[
                ("Cargo.toml", "[package]\nname='sample'"),
                (
                    "src/lib.rs",
                    "mod hidden; mod api; pub use hidden::RealDoc as PublicDoc;",
                ),
                ("src/hidden.rs", &hidden),
                ("src/decoy.rs", DOC),
                ("src/api.rs", caller),
                (
                    "src/hidden/child.rs",
                    "fn f() { let p = super::RealDoc::new(); p.touch(); }",
                ),
                (
                    "benches/run.rs",
                    "fn f() { let p = sample::PublicDoc::new(); p.touch(); }",
                ),
            ],
            true,
            |tree, parsed, ids| {
                verify_pipeline(
                    tree,
                    parsed,
                    ids,
                    "src/hidden/child.rs",
                    Some("src/hidden.rs"),
                );
                verify_pipeline(
                    tree,
                    parsed,
                    ids,
                    "src/api.rs",
                    sibling_access.then_some("src/hidden.rs"),
                );
                verify_pipeline(
                    tree,
                    parsed,
                    ids,
                    "benches/run.rs",
                    (visibility == "pub").then_some("src/hidden.rs"),
                );
            },
        );
    }
}

#[test]
fn inline_module_access_keeps_alias_definition_context_and_terminal_visibility() {
    check_locals("mod hidden { struct Secret; impl Secret { pub fn new() -> Self { Self } pub fn touch(&self) {} }
        pub use self::Secret as PublicDoc; }
        fn f() { let p = hidden::PublicDoc::new(); p.touch(); }", &[]);
    check_locals(
        "mod hidden { pub use crate::b::Doc as Renamed; }
        use hidden::Renamed as Alias;
        fn f() { let p = Alias::new(); p.touch(); }",
        &["b.Doc.new", "b.Doc.touch"],
    );
    check_locals(
        "mod hidden { pub(crate) use crate::b::Doc as Renamed; }
        mod sibling { fn f() { let p = crate::hidden::Renamed::new(); p.touch(); } }",
        &["b.Doc.new", "b.Doc.touch"],
    );
    check_locals(
        "mod hidden { use crate::b::Doc as Renamed; }
        mod sibling { fn f() { let p = crate::hidden::Renamed::new(); p.touch(); } }",
        &[],
    );
}

#[test]
#[ignore = "independent visibility acceptance check; requires rustc on PATH"]
fn module_access_fixtures_agree_with_rustc() {
    for (visibility, caller, public_alias, allowed) in [
        (
            "pub",
            "fn f() { let p = hidden::RealDoc::new(); p.touch(); }",
            false,
            true,
        ),
        (
            "",
            "fn f() { let p = hidden::RealDoc::new(); p.touch(); }",
            false,
            false,
        ),
        (
            "pub(crate)",
            "fn f() { let p = hidden::RealDoc::new(); p.touch(); }",
            false,
            true,
        ),
        (
            "pub(super)",
            "fn f() { let p = hidden::RealDoc::new(); p.touch(); }",
            false,
            true,
        ),
        (
            "pub(self)",
            "fn f() { let p = hidden::RealDoc::new(); p.touch(); }",
            false,
            false,
        ),
        (
            "pub",
            "fn f() { let p = hidden::PublicDoc::new(); p.touch(); }",
            true,
            true,
        ),
        (
            "",
            "fn f() { let p = hidden::PublicDoc::new(); p.touch(); }",
            true,
            false,
        ),
        (
            "pub(crate)",
            "fn f() { let p = hidden::PublicDoc::new(); p.touch(); }",
            true,
            false,
        ),
    ] {
        let doc = DOC.replace("pub struct", &format!("{visibility} struct"));
        let reexport = if public_alias {
            "pub use self::RealDoc as PublicDoc;"
        } else {
            ""
        };
        let source = format!("mod hidden {{ {doc} {reexport} }} {caller}");
        let expected: &[&str] = if allowed {
            &["hidden.RealDoc.new", "hidden.RealDoc.touch"]
        } else {
            &[]
        };
        check_locals(&source, expected);
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("lib.rs");
        std::fs::write(&input, &source).unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--crate-name",
                "access_fixture",
                "--crate-type",
                "lib",
                "--edition=2021",
                "--emit=metadata",
                "--cap-lints=allow",
            ])
            .arg(&input)
            .arg("-o")
            .arg(dir.path().join("lib.rmeta"))
            .output()
            .expect("rustc is required for independent fixture validation");
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), allowed, "{source}\n{diagnostics}");
        if !allowed {
            assert!(
                diagnostics.contains("E0603")
                    || diagnostics.contains("E0364")
                    || diagnostics.contains("E0365"),
                "negative must fail for privacy, not an unrelated compiler error: {diagnostics}"
            );
        }
    }
}

#[test]
fn source_owned_non_call_initializers_keep_their_nominal_and_local_ids() {
    check_locals("fn f() { let p = b::Doc {}; p.touch(); }", &["b.Doc.touch"]);
    check_locals(
        "fn f() { let p = b::Doc::new(); let q = p; q.touch(); }",
        &["b.Doc.new", "b.Doc.touch"],
    );
    check_locals("fn f() { let p = missing::Doc {}; p.touch(); }", &[]);
}

#[test]
fn conflicting_nominal_members_cannot_seed_a_downstream_local_type() {
    check_locals(
        "struct Item; impl Item {
        fn new() -> Self { Self } fn new() -> Self { Self } fn touch(&self) {}
    } fn f() { let p = Item::new(); p.touch(); }",
        &[],
    );
    check_locals(
        "struct Item; impl Item {
        fn make(&self) -> a::Doc { loop {} } fn make(&self) -> b::Doc { loop {} }
    } fn f(p: Item) { let q = p.make(); q.touch(); }",
        &[],
    );
    check_locals(
        "struct Item; impl Item { fn make(&self) -> b::Doc { loop {} } }
        fn f(p: Item) { let q = p.make(); q.touch(); }",
        &["Item.make", "b.Doc.touch"],
    );
}

#[test]
fn private_nominal_methods_and_fields_do_not_leak_yields_to_sibling_callers() {
    check_locals(
        "mod hidden { pub struct Item; impl Item { fn make(&self) -> crate::b::Doc { loop {} } } }
        fn f(p: hidden::Item) { let q = p.make(); q.touch(); }",
        &[],
    );
    check_locals(
        "mod hidden { pub struct Item { inner: crate::b::Doc } }
        fn f(p: hidden::Item) { p.inner.touch(); }",
        &[],
    );
    check_locals("mod hidden { pub struct Item; impl Item { pub fn make(&self) -> crate::b::Doc { loop {} } } }
        fn f(p: hidden::Item) { let q = p.make(); q.touch(); }", &["hidden.Item.make", "b.Doc.touch"]);
}

fn nominal_access_cases() -> Vec<(String, Vec<&'static str>)> {
    let mut cases = Vec::new();
    for (visibility, sibling, outside) in [
        ("", false, false),
        ("pub(self)", false, false),
        ("pub(super)", true, false),
        ("pub(in crate::outer)", true, false),
        ("pub(crate)", true, true),
        ("pub", true, true),
    ] {
        for field in [false, true] {
            for location in 0..4 {
                let access = if field {
                    "p.inner.touch();"
                } else {
                    "let q = p.make(); q.touch();"
                };
                let declaration = if field {
                    format!("pub struct Item {{ {visibility} inner: crate::b::Doc }}")
                } else {
                    format!("pub struct Item; impl Item {{ {visibility} fn make(&self) -> crate::b::Doc {{ loop {{}} }} }}")
                };
                let caller = format!("fn f(p: crate::outer::hidden::Item) {{ {access} }}");
                let source = match location {
                    0 => format!("mod outer {{ pub mod hidden {{ {declaration} {caller} }} }}"),
                    1 => format!("mod outer {{ pub mod hidden {{ {declaration} mod child {{ {caller} }} }} }}"),
                    2 => format!("mod outer {{ pub mod hidden {{ {declaration} }} {caller} }}"),
                    _ => format!("mod outer {{ pub mod hidden {{ {declaration} }} }} {caller}"),
                };
                let allowed =
                    location < 2 || (location == 2 && sibling) || (location == 3 && outside);
                let expected = if !allowed {
                    vec![]
                } else if field {
                    vec!["b.Doc.touch"]
                } else {
                    vec!["outer.hidden.Item.make", "b.Doc.touch"]
                };
                cases.push((source, expected));
            }
        }
    }
    cases
}

#[test]
fn nominal_access_matrix_preserves_legal_yields_and_rejects_sibling_leaks_fresh_and_cold() {
    for (source, expected) in nominal_access_cases() {
        check_locals(&source, &expected);
    }
    check_locals(
        "mod hidden { pub struct Item; impl Item { fn make(&self) -> crate::b::Doc { loop {} } }
        fn first(p: Item) { p.make().touch(); } }
        fn second(p: hidden::Item) { p.make().touch(); }
        mod hidden_again { fn third(p: crate::hidden::Item) { p.make().touch(); } }",
        &["hidden.Item.make", "b.Doc.touch"],
    );
    check_locals(
        "mod hidden { struct Item; impl Item { pub fn touch(&self) {} }
        pub fn make() -> Item { loop {} } }
        fn f() { let p = hidden::make(); p.touch(); }",
        &["hidden.make"],
    );
    check_locals(
        "mod hidden { pub struct Item; #[cfg(feature=\"extra\")] impl Item {
        pub fn make(&self) -> crate::b::Doc { loop {} } } }
        fn f(p: hidden::Item) { p.make().touch(); }",
        &[],
    );
}

#[test]
fn private_methods_follow_the_impls_module_not_the_nominal_owners_module() {
    for (caller, allowed) in [("src/api/child.rs", true), ("src/sibling.rs", false)] {
        check_project_mode(&[("Cargo.toml", "[package]\nname='sample'"),
            ("src/lib.rs", "pub struct RealDoc; mod api; mod sibling;"),
            ("src/api.rs", "impl crate::RealDoc { fn new() -> Self { Self } fn touch(&self) {} } mod child;"),
            (caller, "use crate::RealDoc; fn f() { RealDoc::new().touch(); }")], caller, allowed.then_some("src/api.rs"), true);
    }
}

#[test]
fn qualified_and_forward_renamed_impls_bind_the_same_nominal_id_fresh_and_cold() {
    for target in [
        "crate::RealDoc",
        "super::RealDoc",
        "Renamed",
        "root::RealDoc",
    ] {
        let body = format!(
            "impl {target} {{ pub fn new() -> Self {{ Self }} pub fn touch(&self) {{}} }}
            use crate::RealDoc as Renamed; use crate as root;"
        );
        check_project_mode(
            &[
                ("Cargo.toml", "[package]\nname='sample'"),
                (
                    "src/lib.rs",
                    "pub mod model; pub use model::RealDoc; mod api;",
                ),
                ("src/model.rs", "pub struct RealDoc;"),
                ("src/api.rs", &body),
                (
                    "benches/run.rs",
                    "use sample::RealDoc as AliasDoc; fn f() { AliasDoc::new().touch(); }",
                ),
            ],
            "benches/run.rs",
            Some("src/api.rs"),
            true,
        );
    }
    check_locals(
        "impl Item { pub fn new() -> Self { Self } pub fn touch(&self) {} } struct Item;
        fn f() { Item::new().touch(); }",
        &["Item.new", "Item.touch"],
    );
}

#[test]
fn invalid_impl_owner_evidence_cannot_attach_to_a_local_namesake() {
    for (target, extra, attribute) in [
        ("missing::Item", "", ""),
        ("Alias", "use missing::Item as Alias;", ""),
        (
            "Alias",
            "use left::Item as Alias; use right::Item as Alias;",
            "",
        ),
        ("Item", "", "#[cfg(feature=\"extra\")]"),
    ] {
        check_locals(&format!("struct Item; mod left {{ pub struct Item; }} mod right {{ pub struct Item; }}
            {extra} {attribute} impl {target} {{ pub fn new() -> Self {{ Self }} pub fn touch(&self) {{}} }}
            fn f() {{ Item::new().touch(); }}"), &[]);
    }
}

#[test]
fn blanket_impl_parameter_ids_and_self_applications_preserve_member_yields() {
    check_locals("struct Item<T> { inner: T } impl<Value> Item<Value> { pub fn make(&self) -> Value { loop {} } }
        fn f(p: Item<b::Doc>) { p.make().touch(); }", &["Item.make", "b.Doc.touch"]);
    check_locals("struct Pair<A,B> { a: A, b: B } impl<T,U> Pair<U,T> { pub fn make(&self) -> T { loop {} } }
        fn f(p: Pair<a::Doc,b::Doc>) { p.make().touch(); }", &["Pair.make", "b.Doc.touch"]);
    check_locals(
        "mod model { pub struct Item<T> { pub inner: T } }
        impl<T> crate::model::Item<T> { pub fn make(&self) -> Self { loop {} } }
        fn f(p: model::Item<b::Doc>) { p.make().inner.touch(); }",
        &["Item.make", "b.Doc.touch"],
    );
    check_locals(
        "struct Item<T> { inner: T } impl<T> Item<T> { pub fn make<U>(&self, p: U) -> U { p } }
        fn f(p: Item<a::Doc>, q: b::Doc) { p.make(q).touch(); }",
        &["Item.make", "b.Doc.touch"],
    );
}

#[test]
fn type_alias_impl_owner_probe_requires_a_real_nominal_attachment() {
    check_locals("struct Item; type Alias = Item; impl Alias { pub fn new() -> Self { Self } pub fn touch(&self) {} }
        fn f() { Item::new().touch(); }", &["Alias.new", "Alias.touch"]);
}

#[path = "namespace_alias_owner_tests.rs"]
mod alias_owners;

#[test]
fn inherent_extensions_cannot_be_attached_to_a_foreign_crate_type() {
    check_project_mode(&[("Cargo.toml", "[workspace]\nmembers=['app','api']"),
        ("api/Cargo.toml", "[package]\nname='api'"), ("api/src/lib.rs", "pub struct RealDoc;"),
        ("app/Cargo.toml", "[package]\nname='app'\n[dependencies]\napi={path='../api'}"),
        ("app/src/lib.rs", "use api::RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} }
            fn f() { RealDoc::new().touch(); }")], "app/src/lib.rs", None, true);
}

#[test]
#[ignore = "independent extension owner/generic semantics check; requires rustc on PATH"]
fn extension_owner_fixtures_agree_with_rustc() {
    let cases = [
        ("mod model { pub struct Item; } impl Alias { fn new() -> Self { Self } } use model::Item as Alias; fn f() { Alias::new(); }", true, ""),
        ("mod model { pub struct Item; } impl crate::model::Item { fn new() -> Self { Self } } fn f() { model::Item::new(); }", true, ""),
        ("struct Item<T> { inner: T } impl<Value> Item<Value> { fn make(&self) -> Value { loop {} } } fn f(p: Item<b::Doc>) { p.make().touch(); }", true, ""),
        ("struct Pair<A,B> { a: A, b: B } impl<T,U> Pair<U,T> { fn make(&self) -> T { loop {} } } fn f(p: Pair<a::Doc,b::Doc>) { p.make().touch(); }", true, ""),
        ("mod model { pub struct Item<T> { pub inner: T } } impl<T> crate::model::Item<T> { fn make(&self) -> Self { loop {} } } fn f(p: model::Item<b::Doc>) { p.make().inner.touch(); }", true, ""),
        ("struct Item<T> { inner: T } impl<T> Item<T> { fn make<U>(&self, p: U) -> U { p } } fn f(p: Item<a::Doc>, q: b::Doc) { p.make(q).touch(); }", true, ""),
        ("struct Item; impl missing::Item { fn new() -> Self { Self } }", false, "E0433"),
        ("struct Item; #[cfg(feature=\"extra\")] impl Item { fn new() -> Self { Self } } fn f() { Item::new(); }", false, "E0599"),
        ("mod a1 { pub struct Item; } mod b1 { pub struct Item; } use a1::Item as Alias; use b1::Item as Alias; impl Alias { fn new() -> Self { Self } }", false, "E0252"),
    ];
    for (source, allowed, code) in cases {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("lib.rs");
        std::fs::write(&input, format!("{LOCAL_TYPES} {source}")).unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--crate-name",
                "extension_fixture",
                "--crate-type",
                "lib",
                "--edition=2021",
                "--emit=metadata",
                "--cap-lints=allow",
            ])
            .arg(&input)
            .arg("-o")
            .arg(dir.path().join("lib.rmeta"))
            .output()
            .expect("rustc required");
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), allowed, "{source}\n{diagnostics}");
        if !allowed {
            assert!(diagnostics.contains(code), "{diagnostics}");
        }
    }
    for sibling in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("lib.rs");
        std::fs::write(
            &input,
            if sibling {
                "pub struct Item; mod body; fn f() { Item::new(); }"
            } else {
                "pub struct Item; mod body;"
            },
        )
        .unwrap();
        std::fs::write(dir.path().join("body.rs"), "impl crate::Item { fn new() -> Self { Self } } mod child { fn f() { crate::Item::new(); } }").unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--crate-name",
                "extension_fixture",
                "--crate-type",
                "lib",
                "--edition=2021",
                "--emit=metadata",
                "--cap-lints=allow",
            ])
            .arg(&input)
            .arg("-o")
            .arg(dir.path().join("lib.rmeta"))
            .output()
            .expect("rustc required");
        assert_eq!(
            output.status.success(),
            !sibling,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if sibling {
            assert!(String::from_utf8_lossy(&output.stderr).contains("E0624"));
        }
    }
}

#[test]
fn private_nominal_access_uses_cross_file_requester_ancestry() {
    for (caller, allowed) in [("src/api/child.rs", true), ("src/sibling.rs", false)] {
        check_project_mode(&[("Cargo.toml", "[package]\nname='sample'"),
            ("src/lib.rs", "mod api; mod sibling;"),
            ("src/api.rs", "pub struct RealDoc; impl RealDoc { fn new() -> Self { Self } fn touch(&self) {} } mod child;"),
            (caller, "use crate::api::RealDoc as AliasDoc; fn f() { AliasDoc::new().touch(); }")], caller, allowed.then_some("src/api.rs"), true);
    }
}

#[test]
#[ignore = "independent member privacy acceptance check; requires rustc on PATH"]
fn nominal_member_access_fixtures_agree_with_rustc() {
    for (source, expected) in nominal_access_cases() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("lib.rs");
        std::fs::write(&input, format!("{LOCAL_TYPES} {source}")).unwrap();
        let output = std::process::Command::new("rustc")
            .args([
                "--crate-name",
                "member_access",
                "--crate-type",
                "lib",
                "--edition=2021",
                "--emit=metadata",
                "--cap-lints=allow",
            ])
            .arg(&input)
            .arg("-o")
            .arg(dir.path().join("lib.rmeta"))
            .output()
            .expect("rustc required");
        let diagnostics = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.success(),
            !expected.is_empty(),
            "{source}\n{diagnostics}"
        );
        if expected.is_empty() {
            assert!(
                diagnostics.contains("E0624") || diagnostics.contains("E0616"),
                "{diagnostics}"
            );
        }
    }
}

fn check_locals(body: &str, expected: &[&str]) {
    let source = format!("{LOCAL_TYPES}\n{body}");
    with_project(
        &[
            ("Cargo.toml", "[package]\nname='sample'"),
            ("src/lib.rs", &source),
        ],
        true,
        |tree, parsed, ids| {
            let file = &parsed[0];
            let profiles = super::super::file_context::build_profiles();
            let solver = super::super::semantic_model::SemanticModel::production();
            let (edges, _, _, _) = super::super::pipeline::resolve_one_file(
                file,
                tree,
                &profiles,
                &Default::default(),
                None,
                &solver,
                ids,
                None,
            );
            let actual: Vec<_> = edges
                .iter()
                .filter(|e| {
                    e.2 == "calls"
                        && tree.symbol_by_id(e.1).is_some_and(|s| {
                            matches!(s.name.as_str(), "new" | "touch" | "convert" | "make")
                        })
                })
                .map(|e| e.1)
                .collect();
            let expected: Vec<_> = expected
                .iter()
                .map(|name| {
                    let slot = file
                        .symbols
                        .iter()
                        .position(|s| s.qualified_name == *name)
                        .unwrap_or_else(|| panic!("missing {name}"));
                    ids.row_id(&file.path, slot).unwrap()
                })
                .collect();
            if actual != expected {
                eprintln!("recipes: {:?}", file.flow.lexical.as_ref().unwrap().types);
                for (slot, symbol) in file.symbols.iter().enumerate() {
                    if matches!(
                        symbol.name.as_str(),
                        "Wrapped" | "Holder" | "inner" | "p" | "q"
                    ) {
                        let id = ids.row_id(&file.path, slot).unwrap();
                        eprintln!(
                            "{slot} {id} {} {:?}",
                            symbol.qualified_name,
                            tree.canonical_type_info(id)
                        );
                    }
                }
            }
            assert_eq!(actual, expected, "{body}");
        },
    );
}

#[test]
fn local_pipeline_preserves_same_block_shadowing_and_initializer_order() {
    check_locals(
        "fn f() { let p = a::Doc::new(); let p = p.convert(); p.touch(); }",
        &["a.Doc.new", "a.Doc.convert", "b.Doc.touch"],
    );
}

#[test]
fn local_pipeline_isolates_sibling_blocks_functions_and_missing_shadowed_initializers() {
    check_locals(
        "fn f() { let p = a::Doc::new(); { let p = b::Doc::new(); p.touch(); } p.touch(); }
        fn g() { let p = b::Doc::new(); p.touch(); }",
        &[
            "a.Doc.new",
            "b.Doc.new",
            "b.Doc.touch",
            "a.Doc.touch",
            "b.Doc.new",
            "b.Doc.touch",
        ],
    );
    check_locals(
        "fn f() { let p = a::Doc::new(); let p = missing::Doc::new(); p.touch(); }",
        &["a.Doc.new"],
    );
}

#[test]
fn local_pipeline_binds_bare_imported_factories_and_type_annotations_by_ids() {
    check_locals(
        "fn make() -> b::Doc { loop {} } fn f() { let p = make(); p.touch(); }",
        &["make", "b.Doc.touch"],
    );
    check_locals(
        "mod api { pub fn make() -> super::b::Doc { loop {} } } use api::make;
        fn f() { let p = make(); p.touch(); }",
        &["api.make", "b.Doc.touch"],
    );
    check_locals(
        "use b::Doc as Alias; fn f(p: &Alias) { p.touch(); }",
        &["b.Doc.touch"],
    );
    check_locals(
        "use missing::Doc as Alias; fn f(p: &Alias) { p.touch(); }",
        &[],
    );
    check_locals(
        "use b::Doc as Alias; fn f() { let p: Alias = loop {}; p.touch(); }",
        &["b.Doc.touch"],
    );
    check_locals(
        "fn f() { let p: a::Doc = missing::Doc::new(); p.touch(); }",
        &["a.Doc.touch"],
    );
}

#[test]
fn local_pipeline_keeps_closure_capture_and_type_value_namespaces_separate() {
    check_locals("fn f() { let p = a::Doc::new(); let c = || { p.touch(); let p = b::Doc::new(); p.touch(); }; p.touch(); }",
        &["a.Doc.new", "a.Doc.touch", "b.Doc.new", "b.Doc.touch", "a.Doc.touch"]);
    check_locals(
        "use a::Doc; fn f() { let Doc = b::Doc::new(); Doc::new().touch(); }",
        &["b.Doc.new", "a.Doc.new", "a.Doc.touch"],
    );
}

fn verify_pipeline(
    tree: &Compilation,
    parsed: &[ParsedFile],
    ids: &SymbolIds,
    source_file: &str,
    expected_file: Option<&str>,
) {
    let source = parsed.iter().find(|p| p.path == source_file).unwrap();
    let profiles = super::super::file_context::build_profiles();
    let solver = super::super::semantic_model::SemanticModel::production();
    let (edges, _, _, _) = super::super::pipeline::resolve_one_file(
        source,
        tree,
        &profiles,
        &Default::default(),
        None,
        &solver,
        ids,
        None,
    );
    for name in ["new", "touch"] {
        let actual: Vec<_> = edges
            .iter()
            .filter(|e| tree.symbol_by_id(e.1).is_some_and(|s| s.name == name))
            .map(|e| e.1)
            .collect();
        let expected: Vec<_> = expected_file
            .map(|path| {
                let target = parsed.iter().find(|p| p.path == path).unwrap();
                let slot = target
                    .symbols
                    .iter()
                    .position(|s| s.name == name && s.kind == SymbolKind::Method)
                    .unwrap();
                ids.row_id(path, slot).unwrap()
            })
            .into_iter()
            .collect();
        assert_eq!(
            actual, expected,
            "{source_file}: {name}, expected provider {expected_file:?}"
        );
    }
}

fn verify_project(
    tree: &Compilation,
    parsed: &[ParsedFile],
    ids: &SymbolIds,
    source_file: &str,
    expected_file: Option<&str>,
) {
    let source = parsed.iter().find(|p| p.path == source_file).unwrap();
    let usage = *source
        .flow
        .namespaces
        .as_ref()
        .unwrap()
        .roots
        .values()
        .next()
        .expect("qualified constructor root");
    assert!(
        tree.namespace_use(source_file, usage).is_some(),
        "configured imports must bind through the ID graph, never a legacy fallback"
    );
    let lookup = FileLookup::for_file(tree, source, ids);
    let refs: Vec<_> = source
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls && matches!(r.target_name.as_str(), "new" | "touch"))
        .collect();
    assert_eq!(refs.len(), 2, "the fixture must capture both cascade calls");
    for reference in refs {
        let mut context = testkit::ref_ctx(
            reference,
            &source.symbols[reference.source_symbol_index],
            vec![],
        );
        context.source_symbol_id = ids.row_id(source_file, reference.source_symbol_index);
        let result = super::super::chain::bind_member_access(
            &context,
            &testkit::file_ctx(vec![], None),
            &lookup,
            &crate::languages::rust_lang::RUST_PROFILE,
        );
        if let Some(expected) = expected_file {
            let target = parsed.iter().find(|p| p.path == expected).unwrap();
            let slot = target
                .symbols
                .iter()
                .position(|s| s.name == reference.target_name && s.kind == SymbolKind::Method)
                .unwrap();
            assert_eq!(
                result
                    .unwrap_or_else(|e| panic!("{source_file} {}: {e:?}", reference.target_name))
                    .target_symbol_id,
                ids.row_id(expected, slot).unwrap()
            );
        } else {
            assert!(result.is_err(), "{}: {result:?}", reference.target_name);
        }
    }
}

#[test]
fn renamed_local_module_constructor_uses_exact_owner_and_return_ids_fresh_and_cold() {
    assert_source("mod left { pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} } }
        mod right { pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} } }
        use left::RealDoc as AliasDoc; fn f() { AliasDoc::new().touch(); }", "left.RealDoc.touch");
}

#[test]
fn grouped_namespace_aliases_and_qualified_module_selectors_keep_their_original_targets() {
    for imports in [
        "use right::RealDoc as AliasDoc;",
        "use right::{RealDoc as AliasDoc};",
        "use right::{self as api}; use api::RealDoc as AliasDoc;",
    ] {
        assert_source(&format!("mod left {{ pub struct RealDoc; impl RealDoc {{ pub fn new() -> Self {{ Self }} pub fn touch(&self) {{}} }} }}
            mod right {{ pub struct RealDoc; impl RealDoc {{ pub fn new() -> Self {{ Self }} pub fn touch(&self) {{}} }} }}
            {imports} fn f() {{ AliasDoc::new().touch(); }}"), "right.RealDoc.touch");
    }
    assert_source("mod left { pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} } }
        mod right { pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} } }
        fn f() { right::RealDoc::new().touch(); }", "right.RealDoc.touch");
}

#[test]
fn function_local_renames_shadow_outer_imports_by_scope_not_file_flat_import_order() {
    assert_source("mod left { pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} } }
        mod right { pub struct RealDoc; impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} } }
        use left::RealDoc as AliasDoc; fn f() { use right::RealDoc as AliasDoc; AliasDoc::new().touch(); }", "right.RealDoc.touch");
}

#[test]
fn public_reexports_and_simple_declared_return_heads_use_bound_ids() {
    assert_source("mod left { pub struct RealDoc; impl RealDoc { pub fn new() -> RealDoc { RealDoc } pub fn touch(&self) {} } }
        mod right { pub struct RealDoc; impl RealDoc { pub fn new() -> RealDoc { RealDoc } pub fn touch(&self) {} } }
        mod api { pub use super::right::RealDoc as AliasDoc; }
        use api::AliasDoc; fn f() { AliasDoc::new().touch(); }", "right.RealDoc.touch");
}

fn assert_source(source: &str, target: &str) {
    check_source(source, Some(target));
}

#[test]
fn unresolved_scoped_paths_cannot_borrow_nominal_namesakes() {
    for (extra, body) in [
        ("mod api {} use api::RealDoc as AliasDoc;", "AliasDoc::new().touch();"),
        ("mod api { struct RealDoc; } use api::RealDoc as AliasDoc;", "AliasDoc::new().touch();"),
        ("use left::RealDoc as AliasDoc; use right::RealDoc as AliasDoc;", "AliasDoc::new().touch();"),
        ("use left::RealDoc as AliasDoc;", "use right::*; RealDoc::new().touch();"),
        ("use left::RealDoc as AliasDoc;", "use right::{self, *}; RealDoc::new().touch();"),
        ("#[cfg(feature = \"optional\")] use left::RealDoc as AliasDoc;", "AliasDoc::new().touch();"),
        ("#[cfg(feature = \"optional\")] #[allow(dead_code)] mod api { pub use super::left::RealDoc as AliasDoc; } use api::AliasDoc;", "AliasDoc::new().touch();"),
    ] {
        check_source(&format!("pub struct RealDoc; impl RealDoc {{ pub fn new() -> Self {{ Self }} pub fn touch(&self) {{}} }}
            mod left {{ pub struct RealDoc; impl RealDoc {{ pub fn new() -> Self {{ Self }} pub fn touch(&self) {{}} }} }}
            mod right {{ pub struct RealDoc; impl RealDoc {{ pub fn new() -> Self {{ Self }} pub fn touch(&self) {{}} }} }}
            {extra} fn f() {{ {body} }}"), None);
    }
    check_source(
        "struct RealDoc; impl RealDoc { fn new() -> Self { Self } fn touch(&self) {} }
        fn f<RealDoc>() { RealDoc::new().touch(); }",
        None,
    );
}

#[test]
fn inert_attributes_do_not_hide_nominal_declarations() {
    assert_source(
        "#[allow(dead_code)] mod api { #[derive(Clone)] pub struct RealDoc;
        impl RealDoc { pub fn new() -> Self { Self } pub fn touch(&self) {} } }
        use api::RealDoc as AliasDoc; fn f() { AliasDoc::new().touch(); }",
        "api.RealDoc.touch",
    );
}

fn check_source(source: &str, target: Option<&str>) {
    check_calls(source, target, None);
}

#[test]
fn value_export_selection_keeps_the_namespace_bases_type_domain() {
    check_calls(
        "mod api { pub struct RealDoc; impl RealDoc { pub fn touch(&self) {} }
        pub fn new() -> RealDoc { RealDoc } }
        fn f() { api::new().touch(); }",
        Some("api.RealDoc.touch"),
        Some("api.new"),
    );
}

fn check_calls(source: &str, target: Option<&str>, factory: Option<&str>) {
    let arena = Arc::new(TypeArena::new());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    std::fs::write(&path, source).unwrap();
    let parsed = crate::indexer::parse_file::parse_file_with_arena(
        &crate::walker::WalkedFile {
            relative_path: "a.rs".into(),
            absolute_path: path,
            language: "rust",
        },
        &crate::languages::default_registry(),
        &arena,
    )
    .unwrap();
    let db = crate::Database::open_in_memory().unwrap();
    let (_, ids) = crate::indexer::write::write_parsed_files_with_origin(
        &db,
        std::slice::from_ref(&parsed),
        "internal",
        Some(&arena),
    )
    .unwrap();
    let tree = Compilation::build(std::slice::from_ref(&parsed), &ids, Arc::clone(&arena));
    let expected = target.map(|target| {
        let slot = parsed
            .symbols
            .iter()
            .position(|s| s.qualified_name == target && s.kind == SymbolKind::Method)
            .unwrap();
        let owner = parsed.symbols[slot].parent_index.unwrap();
        let constructor = parsed
            .symbols
            .iter()
            .position(|s| {
                factory.map_or(s.name == "new" && s.parent_index == Some(owner), |name| {
                    s.qualified_name == name
                })
            })
            .unwrap();
        (
            ids.row_id("a.rs", slot).unwrap(),
            ids.row_id("a.rs", constructor).unwrap(),
        )
    });
    tree.persist_type_info(db.conn()).unwrap();
    let restored = Arc::new(TypeArena::new());
    let snapshot: String = db
        .conn()
        .query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    restored.restore_snapshot(&snapshot);
    let mut cold = Compilation::build(&[], &SymbolIds::default(), restored);
    cold.ingest_from_db(db.conn());
    for tree in [&tree, &cold] {
        let lookup = FileLookup::for_file(tree, &parsed, &ids);
        let refs: Vec<_> = parsed
            .refs
            .iter()
            .filter(|r| {
                r.kind == EdgeKind::Calls && matches!(r.target_name.as_str(), "touch" | "new")
            })
            .collect();
        assert_eq!(refs.iter().filter(|r| r.target_name == "touch").count(), 1);
        assert!(refs.iter().any(|r| r.target_name == "new"));
        for reference in refs {
            let mut context = testkit::ref_ctx(
                reference,
                &parsed.symbols[reference.source_symbol_index],
                vec![],
            );
            context.source_symbol_id = ids.row_id("a.rs", reference.source_symbol_index);
            let result = super::super::chain::bind_member_access(
                &context,
                &testkit::file_ctx(vec![], None),
                &lookup,
                &crate::languages::rust_lang::RUST_PROFILE,
            );
            if let Some((method, constructor)) = expected {
                assert_eq!(
                    result
                        .unwrap_or_else(|cause| panic!("{reference:?}: {cause:?}\n{source}"))
                        .target_symbol_id,
                    if reference.target_name == "new" {
                        constructor
                    } else {
                        method
                    }
                );
            } else {
                assert!(
                    result.is_err(),
                    "must not borrow a target for {reference:?}: {result:?}\n{source}"
                );
            }
        }
    }
}
