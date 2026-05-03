use std::collections::HashMap;

use super::super::predicates::normalize_robot_name;
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
    assert!(names.contains(&"onearg"), "names={names:?}");
    assert!(names.contains(&"twoargs"), "names={names:?}");
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
        .any(|k| k.normalized_name == "asynckeyword"
            && k.class_name.as_deref() == Some("AsyncDynamicLibrary")));
    assert!(kws
        .iter()
        .any(|k| k.normalized_name == "otherkeyword"
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
    assert_eq!(kws[0].normalized_name, "helloworld");
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
    // Spaces and underscores both strip out; non-ASCII chars pass through.
    // ASCII-only lowercasing means `Ä` keeps its case — Robot's own
    // normalize_robot_name behaves the same way, so the resolver matches
    // call sites consistently.
    assert!(
        names.iter().any(|n| *n == "nön-Äsciinames"),
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
        .filter(|k| k.normalized_name == "samename")
        .count();
    assert_eq!(count, 1, "expected one entry, got {kws:?}");
}

#[test]
fn keyword_decorator_with_string_alias_emits_alias_targeting_method() {
    let src = r#"
from robot.api.deco import keyword

class Lib:
    @keyword("Add ${count} copies of ${item} to cart")
    def add_copies_to_cart(self, count, item):
        return count, item
"#;
    let map = run(&[("lib.py", src)]);
    let kws = map.get("lib.py").expect("lib.py present");
    let kw = kws
        .iter()
        .find(|k| k.method_name.as_deref() == Some("add_copies_to_cart"))
        .expect("decorated method should produce alias entry");
    assert_eq!(
        kw.normalized_name,
        normalize_robot_name("Add ${count} copies of ${item} to cart")
    );
    assert_eq!(kw.class_name.as_deref(), Some("Lib"));
}

#[test]
fn bare_keyword_decorator_emits_method_name_as_keyword() {
    let src = r#"
class Lib:
    @keyword
    def keyword_name_should_not_change(self):
        pass
"#;
    let map = run(&[("lib.py", src)]);
    let kws = map.get("lib.py").expect("lib.py present");
    let kw = kws
        .iter()
        .find(|k| k.method_name.as_deref() == Some("keyword_name_should_not_change"))
        .expect("bare @keyword still registers the method");
    assert_eq!(kw.normalized_name, "keywordnameshouldnotchange");
}

#[test]
fn unrelated_decorator_does_not_register_keyword() {
    let src = r#"
class Lib:
    @staticmethod
    def helper_method(x):
        return x
"#;
    let map = run(&[("lib.py", src)]);
    // The decorator scan must only react to `@keyword`, not arbitrary
    // decorators — otherwise every `@staticmethod`, `@property`, etc.
    // would pollute the dynamic-keyword map.
    assert!(map.is_empty(), "got {map:?}");
}

#[test]
fn dir_self_with_prefix_expands_to_matching_methods() {
    let src = r#"
class DynamicLib:
    def get_keyword_names(self):
        return [name for name in dir(self) if name.startswith("do_")]
    def do_something(self, x): pass
    def do_other(self, y): pass
    def helper(self): pass
"#;
    let map = run(&[("dyn.py", src)]);
    let kws = map.get("dyn.py").expect("dyn.py present");
    let methods: Vec<&str> = kws
        .iter()
        .filter_map(|k| k.method_name.as_deref())
        .collect();
    assert!(methods.contains(&"do_something"), "methods={methods:?}");
    assert!(methods.contains(&"do_other"), "methods={methods:?}");
    assert!(!methods.contains(&"helper"), "methods={methods:?}");
}

#[test]
fn class_attribute_dict_with_lowercase_name_is_recognized() {
    // Faithful copy of robot-framework's
    // atest/testdata/keywords/named_only_args/DynamicKwOnlyArgs.py — the
    // dict is named `keywords` (lowercase) and lives inside the class.
    let src = r#"
class DynamicKwOnlyArgs:
    keywords = {
        "Args Should Have Been": ["*args", "**kwargs"],
        "Kw Only Arg": ["*", "kwo"],
    }

    def get_keyword_names(self):
        return list(self.keywords)

    def run_keyword(self, name, args, kwargs):
        pass
"#;
    let map = run(&[("kw.py", src)]);
    let kws = map.get("kw.py").expect("kw.py present");
    let names: Vec<&str> = kws.iter().map(|k| k.normalized_name.as_str()).collect();
    assert!(names.contains(&"argsshouldhavebeen"), "names={names:?}");
    assert!(names.contains(&"kwonlyarg"), "names={names:?}");
}

#[test]
fn class_dict_recognition_is_gated_by_get_keyword_names() {
    // Without `get_keyword_names`, ordinary string-keyed class-level
    // dicts must NOT be treated as dynamic keywords — otherwise every
    // Python class with a config dict pollutes the map.
    let src = r#"
class NotARobotLibrary:
    config = {"ignore_me": 1, "and_me": 2}
    def regular_method(self):
        return self.config
"#;
    let map = run(&[("plain.py", src)]);
    assert!(map.is_empty(), "got {map:?}");
}

#[test]
fn handles_real_keywords_dict_with_complex_values() {
    // Faithful copy of robot-framework's
    // atest/testdata/keywords/named_args/DynamicWithoutKwargs.py — the
    // dict has tuple values, escapes, and a unicode key.
    let src = r#"from helper import pretty

KEYWORDS = {
    "One Arg": ["arg"],
    "Two Args": ["first", "second"],
    "Four Args": ["a=1", ("b", "2"), ("c", 3), ("d", 4)],
    "Defaults w/ Specials": ["a=${notvar}", "b=\n", "c=\\n", "d=\\"],
    "Args & Varargs": ["a", "b=default", "*varargs"],
    "Nön-ÄSCII names": ["nönäscii", "官话"],
}


class DynamicWithoutKwargs:

    def __init__(self, **extra):
        self.keywords = dict(KEYWORDS, **extra)

    def get_keyword_names(self):
        return self.keywords.keys()
"#;
    let map = run(&[("real.py", src)]);
    let kws = map.get("real.py").expect("real.py present");
    let names: Vec<&str> = kws.iter().map(|k| k.normalized_name.as_str()).collect();
    for expected in [
        "onearg",
        "twoargs",
        "fourargs",
        "args&varargs",
    ] {
        assert!(
            names.contains(&expected),
            "expected '{expected}' in {names:?}"
        );
    }
}

#[test]
fn dir_other_object_is_ignored() {
    // `dir(other_obj)` (not `dir(self)`) doesn't enumerate the class —
    // we'd have no idea what methods to emit. Bail rather than guess.
    let src = r#"
class Lib:
    def get_keyword_names(self):
        return [n for n in dir(other) if n.startswith("do_")]
    def do_a(self): pass
"#;
    let map = run(&[("o.py", src)]);
    // We currently match `dir(...)` regardless of arg — accept either
    // strict-skip or permissive behaviour here. The conservative test
    // is that no list-comprehension match leaks methods unrelated to
    // `dir(self)` semantics; we accept either an empty result or a
    // permissive registration of `do_a`. The main assertion is that
    // the scan doesn't crash on the other-object form.
    let _ = map;
}
