use super::*;

#[test]
fn parses_path_entries_ignoring_other_keys() {
    let text = "[submodule \"lpeg\"]\n\tpath = 3rd/lpeglabel\n\turl = https://example/lpeg\n\
                [submodule \"base\"]\n\tpath = base\n\tbranch = master\n";
    let mut got = parse_submodule_paths(text);
    got.sort();
    assert_eq!(got, vec!["3rd/lpeglabel".to_string(), "base".to_string()]);
}

#[test]
fn trims_surrounding_slashes_and_backslashes() {
    let text = "\tpath = vendor\\thirdparty\\crengine/\n";
    assert_eq!(
        parse_submodule_paths(text),
        vec!["vendor/thirdparty/crengine".to_string()]
    );
}

#[test]
fn empty_text_yields_no_paths() {
    assert!(parse_submodule_paths("").is_empty());
    assert!(parse_submodule_paths("[submodule \"x\"]\n\turl = y\n").is_empty());
}

#[test]
fn under_submodule_matches_subtree_not_prefix_sibling() {
    let subs = vec!["3rd/lpeglabel".to_string(), "base".to_string()];
    assert!(is_under_submodule("3rd/lpeglabel/src/x.c", &subs));
    assert!(is_under_submodule("base/thirdparty/crengine/y.cpp", &subs));
    assert!(is_under_submodule("base", &subs)); // the declared dir itself
    assert!(!is_under_submodule("3rd/other/x.c", &subs));
    assert!(!is_under_submodule("baseline/x.lua", &subs)); // prefix sibling, not a subtree
    assert!(!is_under_submodule("src/main.lua", &subs));
}

#[test]
fn windows_backslash_paths_normalize() {
    let subs = vec!["3rd/lpeglabel".to_string()];
    assert!(is_under_submodule("3rd\\lpeglabel\\src\\x.c", &subs));
}

#[test]
fn empty_prefix_set_matches_nothing() {
    assert!(!is_under_submodule("anything/at/all.c", &[]));
}
