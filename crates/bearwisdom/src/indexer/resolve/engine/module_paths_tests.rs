use super::*;

#[test]
fn exact_relative_paths_keep_esm_substitution_and_missing_directories() {
    let rules = PathRules {
        extensions: vec![".ts".into()],
        substitutions: vec![(".js".into(), vec![".ts".into(), ".js".into()])],
        directory_entry: "index".into(),
    };
    let files = [
        ("a/model.ts", 71),
        ("b/model.ts", 72),
        ("b/model.js", 73),
        ("dir/index.ts", 74),
    ];
    let lookup = |path: &str| {
        files
            .iter()
            .find(|(file, _)| *file == path)
            .map(|(_, id)| *id)
    };
    assert_eq!(
        find(
            &relative_base("b/use.ts", "./model.js").unwrap(),
            &rules,
            lookup
        ),
        Some(72)
    );
    assert_eq!(
        find(
            &relative_base("missing/use.ts", "./model").unwrap(),
            &rules,
            lookup
        ),
        None
    );
    assert_eq!(
        find(&relative_base("root.ts", "./dir").unwrap(), &rules, lookup),
        Some(74)
    );
    assert_eq!(
        relative_base("b/use.ts", "../a/model"),
        Some("a/model".into())
    );
    assert_eq!(
        normalize("../a.ts"),
        "../a.ts",
        "escaping the project cannot become a project-root path"
    );
}
