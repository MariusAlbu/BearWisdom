use super::*;

#[test]
fn markers_use_utf8_bytes_not_character_counts() {
    let marked = strip_markers(
        FixtureFileId(7),
        "// π\n/*@decl:1:function*/function f() { /*@ref:2*/f(); /*@ref:3*/f(); }",
    )
    .unwrap();
    assert_eq!(marked.source, "// π\nfunction f() { f(); f(); }");
    assert_eq!(
        marked.declarations[&1],
        DeclarationSite {
            file: FixtureFileId(7),
            line: 1,
            col: 0,
            kind: SymbolKind::Function,
        }
    );
    assert_eq!(
        &marked.source[marked.refs[&2].byte_offset as usize..],
        "f(); f(); }"
    );
    assert!(marked.refs[&2].byte_offset < marked.refs[&3].byte_offset);
}

#[test]
fn invalid_and_duplicate_markers_are_errors() {
    for input in [
        "/*@ref:1",
        "/*@ref:1*/f(); /*@ref:1*/g();",
        "/*@decl:1*/",
        "/*@unknown:1*/",
    ] {
        assert!(strip_markers(FixtureFileId(1), input).is_err());
    }
}

#[test]
fn configuration_changes_revision_without_relabelling_targets() {
    let fixture = Fixture {
        path: "lib.rs",
        language: "rust",
        marked_source: "/*@decl:1:function*/fn a() {} fn f() { /*@ref:11*/a(); }",
    };
    let files = [fixture];
    let labels = [(11, Some(1))];
    let config = "[package]\nname='first'\n[lib]\npath='lib.rs'";
    let other = "[package]\nname='second'\n[lib]\npath='lib.rs'";
    let first =
        run_configured_selectors(&files, &labels, &[("Cargo.toml", config)], false).unwrap();
    let cold = run_configured_selectors(&files, &labels, &[("Cargo.toml", config)], true).unwrap();
    let second =
        run_configured_selectors(&files, &labels, &[("Cargo.toml", other)], false).unwrap();
    assert_eq!(first, cold);
    assert_ne!(first.revision, second.revision);
    assert_eq!(first.references, second.references);
    assert_eq!(first.counts.correct, 1);
    assert_ne!(
        first.revision,
        run_selectors(&files, &labels).unwrap().revision
    );
}

#[test]
fn fixture_paths_cannot_escape_or_replace_configuration() {
    for path in ["../outside.rs", "src/../../outside.rs", ""] {
        let files = [Fixture {
            path,
            language: "rust",
            marked_source: "",
        }];
        assert!(run_selectors(&files, &[]).is_err());
    }
    let files = [Fixture {
        path: "Cargo.toml",
        language: "rust",
        marked_source: "",
    }];
    assert!(run_configured_selectors(&files, &[], &[("Cargo.toml", "")], false).is_err());
    assert!(run_configured_selectors(&[], &[], &[("../outside.toml", "")], false).is_err());
    assert!(run_configured_selectors(&[], &[], &[], false).is_err());
}
