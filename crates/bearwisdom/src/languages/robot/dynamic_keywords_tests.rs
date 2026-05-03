use std::collections::HashMap;

use super::*;

fn run(files: &[(&str, &str)]) -> RobotDynamicKeywordMap {
    let map: HashMap<&str, &str> = files.iter().copied().collect();
    let paths: Vec<&str> = files.iter().map(|(p, _)| *p).collect();
    build_robot_dynamic_keyword_map(&paths, |p| map.get(p).map(|s| s.to_string()))
}

#[test]
fn module_level_keywords_dict_emits_normalized_keys() {
    let src = r#"
KEYWORDS = {
    "One Arg":  ["arg"],
    "Two Args": ["a", "b"],
}

class DynamicWithoutKwargs:
    def __init__(self, **extra):
        self.keywords = dict(KEYWORDS, **extra)
    def get_keyword_names(self):
        return self.keywords.keys()
"#;
    let map = run(&[("dyn.py", src)]);
    let kws = map.get("dyn.py").expect("dyn.py present");
    let names: Vec<&str> = kws.iter().map(|k| k.normalized_name.as_str()).collect();
    assert!(names.contains(&"one_arg"), "names={names:?}");
    assert!(names.contains(&"two_args"), "names={names:?}");
    // Module-level dict has no class scope.
    assert!(kws.iter().all(|k| k.class_name.is_none()));
}

#[test]
fn get_keyword_names_list_literal_emits_per_class() {
    let src = r#"
class AsyncDynamicLibrary:
    async def get_keyword_names(self):
        return ["async_keyword", "Other Keyword"]
"#;
    let map = run(&[("async.py", src)]);
    let kws = map.get("async.py").expect("async.py present");
    assert!(kws
        .iter()
        .any(|k| k.normalized_name == "async_keyword"
            && k.class_name.as_deref() == Some("AsyncDynamicLibrary")));
    assert!(kws
        .iter()
        .any(|k| k.normalized_name == "other_keyword"
            && k.class_name.as_deref() == Some("AsyncDynamicLibrary")));
}

#[test]
fn class_scoped_keywords_dict_records_class_name() {
    let src = r#"
class Inner:
    KEYWORDS = {
        "Hello World": [],
    }
"#;
    let map = run(&[("nested.py", src)]);
    let kws = map.get("nested.py").expect("nested.py present");
    assert_eq!(kws.len(), 1);
    assert_eq!(kws[0].normalized_name, "hello_world");
    assert_eq!(kws[0].class_name.as_deref(), Some("Inner"));
}

#[test]
fn dynamic_keywords_skips_files_without_either_pattern() {
    let src = r#"
def passing_handler(*args):
    return ", ".join(args)

class Plain:
    def regular_method(self):
        return 42
"#;
    let map = run(&[("plain.py", src)]);
    assert!(map.is_empty(), "plain libraries should not appear in the map");
}

#[test]
fn unicode_keyword_names_normalize_consistently() {
    let src = r#"
KEYWORDS = {
    "Nön-ÄSCII names": [],
    "官话": [],
}
"#;
    let map = run(&[("u.py", src)]);
    let kws = map.get("u.py").expect("u.py present");
    let names: Vec<&str> = kws.iter().map(|k| k.normalized_name.as_str()).collect();
    // Spaces become underscores, ASCII letters get lowercased; non-ASCII
    // chars pass through unchanged (so `Ä` stays uppercase). Robot's own
    // normalize_robot_name behaves the same way, so the resolver's call-
    // site form matches.
    assert!(
        names.iter().any(|n| *n == "nön-Äscii_names"),
        "names={names:?}"
    );
    assert!(names.iter().any(|n| n == &"官话"));
}

#[test]
fn missing_file_does_not_crash() {
    let map = build_robot_dynamic_keyword_map(&["does/not/exist.py"], |_| None);
    assert!(map.is_empty());
}

#[test]
fn duplicate_keyword_in_two_shapes_dedups() {
    // KEYWORDS dict and the list literal both expose "Same Name".
    let src = r#"
class Box:
    KEYWORDS = {"Same Name": []}
    def get_keyword_names(self):
        return ["Same Name"]
"#;
    let map = run(&[("dup.py", src)]);
    let kws = map.get("dup.py").expect("dup.py present");
    let count = kws
        .iter()
        .filter(|k| k.normalized_name == "same_name")
        .count();
    assert_eq!(count, 1, "expected one entry, got {kws:?}");
}
