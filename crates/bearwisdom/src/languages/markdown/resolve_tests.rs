use super::*;

#[test]
fn lexical_normalize_collapses_parent_dirs() {
    let raw = Path::new("docs/intro/../CHANGELOG");
    let n = lexical_normalize(raw);
    assert_eq!(n, PathBuf::from("docs/CHANGELOG"));
}

#[test]
fn lexical_normalize_handles_multi_step_parent() {
    let raw = Path::new("docs/api/v2/../../../README");
    let n = lexical_normalize(raw);
    assert_eq!(n, PathBuf::from("README"));
}

#[test]
fn lexical_normalize_keeps_unmatched_parent_at_root() {
    // No segment to consume — `..` survives the normalization. Real
    // links like this don't resolve, but the normalizer must not panic
    // or return something fictitious.
    let raw = Path::new("../../outside");
    let n = lexical_normalize(raw);
    assert_eq!(n, PathBuf::from("../../outside"));
}

#[test]
fn path_candidates_extensionless_target_probes_markdown_extensions() {
    let cands = path_candidates(Path::new("docs/overview"));
    let names: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(names.contains(&"docs/overview".to_string()));
    assert!(names.contains(&"docs/overview.md".to_string()));
    assert!(names.contains(&"docs/overview.mdx".to_string()));
    assert!(names.contains(&"docs/overview/index.md".to_string()));
    assert!(names.contains(&"docs/overview/README.md".to_string()));
}

#[test]
fn path_candidates_extensioned_target_does_not_re_extend() {
    // `[link](./CHANGELOG.md)` already carries an extension; we should
    // try `CHANGELOG.md` directly, plus `index.*` / `README.*` variants
    // (in case the link points at a directory containing such), but
    // should NOT produce `CHANGELOG.md.md`.
    let cands = path_candidates(Path::new("CHANGELOG.md"));
    let names: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(names.contains(&"CHANGELOG.md".to_string()));
    assert!(!names.iter().any(|n| n.ends_with(".md.md")));
}

#[test]
fn path_candidates_translation_suffix_appends_md() {
    // dockerfile-nodebestpractices links translated articles like
    // `[fr](./eslint_prettier.french)` against the on-disk file
    // `eslint_prettier.french.md`. Path::extension treats `.french` as
    // an extension (it's the part after the last dot in the filename),
    // but it's a translation tag, not a file extension. The candidate
    // generator must APPEND `.md` rather than REPLACE the suffix —
    // otherwise `with_extension` silently rewrites `foo.french` to
    // `foo.md` and the real file is never probed.
    let cands = path_candidates(Path::new("eslint_prettier.french"));
    let names: Vec<String> = cands
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(
        names.contains(&"eslint_prettier.french.md".to_string()),
        "missing eslint_prettier.french.md from candidates: {names:?}"
    );
    // The replace-style behaviour (which would produce `eslint_prettier.md`)
    // must NOT be used.
    assert!(!names.contains(&"eslint_prettier.md".to_string()));
}
