use super::*;
use crate::indexer::resolve::engine::testkit::{import, sym, Lookup};

/// A type row named `Zqrepo`, declared into `scope` and owned by `package_id`.
fn zqrepo(id: i64, qname: &str, file: &str, scope: &str, package_id: Option<i64>) -> Symbol {
    let mut s = sym(id, "Zqrepo", qname, "class", file);
    s.scope_path = Some(scope.to_string());
    s.package_id = package_id;
    s.visibility = Some("public".to_string());
    s
}

/// The use site: a java file in package `app`, optionally declaring it.
fn caller(file_namespace: Option<&str>) -> FileContext {
    FileContext {
        file_path: "src/main/java/app/App.java".into(),
        language: "java".into(),
        imports: Vec::new(),
        file_namespace: file_namespace.map(str::to_string),
    }
}

/// A use site whose only evidence is its import lines.
fn importing_caller(language: &str, file_path: &str, modules: &[&str]) -> FileContext {
    FileContext {
        file_path: file_path.into(),
        language: language.into(),
        imports: modules
            .iter()
            .copied()
            .map(|m| import("Zqrepo", Some(m)))
            .collect(),
        file_namespace: None,
    }
}

/// An external `Zqrepo` row declared into `namespace`, its file path mirroring
/// that namespace under a dependency root.
fn external_zqrepo(id: i64, namespace: &str, root: &str) -> Symbol {
    let path = format!("{root}{}/Zqrepo.java", namespace.replace('.', "/"));
    zqrepo(id, &format!("{namespace}.Zqrepo"), &path, namespace, None)
}

#[test]
fn same_source_namespace_outranks_same_workspace_package() {
    // Three declarations share the simple name. The caller's own namespace
    // names only the first; the second sits in the caller's BUILD unit and the
    // third is an external homonym. Source namespace must win.
    let own = zqrepo(
        1,
        "app.Zqrepo",
        "src/main/java/app/Zqrepo.java",
        "app",
        None,
    );
    let same_pkg = zqrepo(
        2,
        "other.Zqrepo",
        "src/main/java/other/Zqrepo.java",
        "other",
        Some(1),
    );
    let external = zqrepo(
        3,
        "java.x.Zqrepo",
        "ext:java:jdk/java/x/Zqrepo.java",
        "java.x",
        None,
    );
    let lookup = Lookup::new();
    let cands = [&own, &same_pkg, &external];

    let picked = pick_ranked_candidate(&caller(Some("app")), Some(1), &lookup, &cands)
        .expect("the caller's own namespace clears the rank margin");
    assert_eq!(picked.id, own.id);
}

#[test]
fn two_candidates_in_the_caller_namespace_still_decline() {
    // The namespace term is evidence, not a tie-break: when it applies to both
    // candidates the field stays ambiguous and the pick declines.
    let first = zqrepo(1, "app.Zqrepo", "src/main/java/app/A.java", "app", None);
    let second = zqrepo(2, "app.Zqrepo", "src/main/java/app/B.java", "app", None);
    let lookup = Lookup::new();
    let cands = [&first, &second];

    assert!(pick_ranked_candidate(&caller(Some("app")), None, &lookup, &cands).is_none());
}

#[test]
fn no_declared_namespace_leaves_scores_unchanged() {
    // A language whose extractor emits no namespace symbol scores exactly as
    // it did without the term: path proximity alone cannot clear the margin.
    let near = zqrepo(
        1,
        "app.Zqrepo",
        "src/main/java/app/Zqrepo.java",
        "app",
        None,
    );
    let far = zqrepo(
        2,
        "other.Zqrepo",
        "src/main/java/other/Zqrepo.java",
        "other",
        None,
    );
    let lookup = Lookup::new();
    let cands = [&near, &far];

    assert!(pick_ranked_candidate(&caller(None), None, &lookup, &cands).is_none());
}

#[test]
fn path_proximity_score_shared_dir() {
    assert_eq!(
        path_proximity_score("src/views/Page.ts", "src/views/Helper.ts"),
        20
    );
}

#[test]
fn path_proximity_score_no_overlap() {
    assert_eq!(path_proximity_score("src/a/X.ts", "lib/b/Y.ts"), 0);
}

#[test]
fn path_proximity_score_partial_overlap() {
    assert_eq!(path_proximity_score("src/a/b/X.ts", "src/a/c/Y.ts"), 20);
}

#[test]
fn implicit_wildcard_namespace_scopes_ranked_pick() {
    // Two external classes share the bare name; only one sits under a
    // manifest-declared implicit namespace (`<Using Include="Xunit"/>`).
    // The scoped one must win the ranked pick even though the decoy has
    // the lower id (the insertion-order/first-winner fallback).
    let decoy = sym(
        1,
        "Assert",
        "NetTopologySuite.Utilities.Assert",
        "class",
        "ext:dotnet-type:/nts.dll!!nts!!NetTopologySuite.Utilities.Assert",
    );
    let scoped = sym(
        2,
        "Assert",
        "Xunit.Assert",
        "class",
        "ext:dotnet-type:/xa.dll!!xa!!Xunit.Assert",
    );
    let lookup = Lookup::new()
        .with(decoy.clone())
        .with(scoped.clone())
        .with_implicit_namespaces(&["Xunit"]);
    let fc = FileContext {
        file_path: "tests/ParserTests.cs".into(),
        language: "csharp".into(),
        imports: Vec::new(),
        file_namespace: None,
    };
    let cands = [&decoy, &scoped];
    let picked = pick_ranked_candidate(&fc, None, &lookup, &cands)
        .expect("scoped candidate must win by more than the rank margin");
    assert_eq!(picked.qualified_name, "Xunit.Assert");
}

#[test]
fn an_import_naming_the_candidate_scopes_the_pick() {
    // Both homonyms are external and equally far from the use site; the only
    // evidence separating them is the import line, which names one of them.
    let imported = external_zqrepo(1, "com.example.lib", "ext:mvn:");
    let decoy = external_zqrepo(2, "com.other", "ext:mvn:");
    let lookup = Lookup::new();
    let cands = [&imported, &decoy];
    let fc = importing_caller(
        "java",
        "src/main/java/app/App.java",
        &["com.example.lib.Zqrepo"],
    );

    let picked = pick_ranked_candidate(&fc, None, &lookup, &cands)
        .expect("the import names exactly one of the homonyms");
    assert_eq!(picked.id, imported.id);
}

#[test]
fn a_wildcard_import_of_the_declaring_namespace_scopes_the_pick() {
    let imported = external_zqrepo(1, "com.example.lib", "ext:mvn:");
    let decoy = external_zqrepo(2, "com.other", "ext:mvn:");
    let lookup = Lookup::new();
    let cands = [&imported, &decoy];
    let fc = importing_caller("java", "src/main/java/app/App.java", &["com.example.lib"]);

    let picked = pick_ranked_candidate(&fc, None, &lookup, &cands)
        .expect("the wildcard import names one homonym's declaring namespace");
    assert_eq!(picked.id, imported.id);
}

#[test]
fn an_import_naming_neither_candidate_leaves_the_field_ambiguous() {
    let first = external_zqrepo(1, "com.example.lib", "ext:mvn:");
    let second = external_zqrepo(2, "com.other.lib", "ext:mvn:");
    let lookup = Lookup::new();
    let cands = [&first, &second];
    let fc = importing_caller("java", "src/main/java/app/App.java", &["com.third.Other"]);

    assert!(pick_ranked_candidate(&fc, None, &lookup, &cands).is_none());
}

#[test]
fn a_relative_module_specifier_names_no_index_namespace() {
    // A path specifier is not a namespace: it must not score a candidate whose
    // index qname happens to lead with the same segment.
    let first = zqrepo(1, "lib.Zqrepo", "src/lib/Zqrepo.ts", "lib", None);
    let second = zqrepo(2, "other.Zqrepo", "src/other/Zqrepo.ts", "other", None);
    let lookup = Lookup::new();
    let cands = [&first, &second];
    let fc = importing_caller("typescript", "src/app.ts", &["./lib"]);

    assert!(pick_ranked_candidate(&fc, None, &lookup, &cands).is_none());
}

#[test]
fn the_language_prelude_namespace_scopes_the_pick() {
    // No import names either homonym; the language's own compiler-implicit
    // prelude is the only scope evidence the use site carries.
    let prelude = external_zqrepo(1, "java.lang", "ext:jdk:");
    let decoy = external_zqrepo(2, "org.xpath", "ext:jdk:");
    let lookup = Lookup::new();
    let cands = [&prelude, &decoy];

    let picked = pick_ranked_candidate(&caller(None), None, &lookup, &cands)
        .expect("the prelude namespace clears the rank margin");
    assert_eq!(picked.id, prelude.id);
}

#[test]
fn a_language_declaring_no_prelude_gains_no_term() {
    // TypeScript declares no compiler-implicit namespace: the term stays inert
    // and the field is exactly as ambiguous as it was.
    let first = zqrepo(1, "java.lang.Zqrepo", "src/a/Zqrepo.ts", "java.lang", None);
    let second = zqrepo(2, "org.xpath.Zqrepo", "src/b/Zqrepo.ts", "org.xpath", None);
    let lookup = Lookup::new();
    let cands = [&first, &second];
    let fc = importing_caller("typescript", "src/app.ts", &[]);

    assert!(pick_ranked_candidate(&fc, None, &lookup, &cands).is_none());
}
