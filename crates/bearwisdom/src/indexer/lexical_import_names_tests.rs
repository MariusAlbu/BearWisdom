use super::*;

const FORMS: Forms = Forms {
    statements: &[("use_declaration", "argument")],
    paths: &[("scoped_identifier", "name", "path")],
    groups: &[("scoped_use_list", "path", "list")],
    lists: &["use_list"],
    renames: &[("use_as_clause", "alias")],
    identifiers: &["identifier"],
    self_leaf: "self",
    wildcards: &["use_wildcard"],
    discarded: &["_"],
    trivia: &["line_comment", "block_comment"],
};

fn names(source: &str) -> (Vec<String>, bool, bool) {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let names = capture(
        tree.root_node().named_child(0).unwrap(),
        source.as_bytes(),
        &FORMS,
    )
    .unwrap();
    (
        names.exposed.into_iter().map(str::to_owned).collect(),
        names.wildcard,
        names.unknown,
    )
}

#[test]
fn nested_paths_and_aliases_expose_only_the_bound_names() {
    let (names, wildcard, unknown) =
        names("use a::b::{self, c, d::{self as dd, /* trivia */ e::F, *}, g::H as _, X as Y};");
    assert_eq!(names, ["b", "c", "dd", "F", "Y"]);
    assert!(wildcard);
    assert!(!unknown);
}

#[test]
fn empty_and_discarded_imports_do_not_create_names_or_unknown_barriers() {
    for source in ["use a::{};", "use a::B as _;", "use a::{self as _};"] {
        assert_eq!(names(source), (vec![], false, false), "{source}");
    }
}

#[test]
fn missing_profile_arguments_remain_unknown() {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let source = "use a::B;";
    let tree = parser.parse(source, None).unwrap();
    let forms = Forms {
        statements: &[("use_declaration", "absent_argument")],
        ..FORMS
    };
    let captured = capture(
        tree.root_node().named_child(0).unwrap(),
        source.as_bytes(),
        &forms,
    )
    .unwrap();
    assert!(captured.unknown);
    assert!(captured.exposed.is_empty());
}
