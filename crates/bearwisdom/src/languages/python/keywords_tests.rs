use super::*;

#[test]
fn builtin_names_excludes_generic_type_params() {
    // GenericParamRule binds these; builtin_skip must never see them.
    for name in ["T", "U", "K", "V", "E", "R", "S"] {
        assert!(
            !BUILTIN_NAMES.contains(&name),
            "{name} is a generic-param placeholder, not a builtin"
        );
    }
}

#[test]
fn builtin_names_excludes_stdlib_with_source() {
    // typing / unittest.mock / dataclasses / functools all have real `.py`
    // source the cpython-stdlib walker indexes — they must never be folded
    // into the C-implemented builtin set.
    for name in ["Optional", "Union", "MagicMock", "Mock", "dataclass", "field", "wraps"] {
        assert!(!BUILTIN_NAMES.contains(&name), "{name} has real stdlib source");
    }
}

#[test]
fn builtin_names_covers_the_evidenced_leak_surface() {
    // Names the python-superset corpus census showed leaking as unresolved
    // Calls and TypeRef targets before builtin_skip was wired.
    for name in [
        "len", "str", "isinstance", "dict", "list", "tuple", "set", "object", "type", "super",
        "getattr", "range", "print", "sorted", "zip", "classmethod", "staticmethod", "frozenset",
        "Exception", "ValueError", "TypeError", "KeyError", "IndexError", "AttributeError",
        "RuntimeError", "OSError",
    ] {
        assert!(BUILTIN_NAMES.contains(&name), "missing evidenced builtin: {name}");
    }
}

#[test]
fn builtin_names_is_a_superset_of_keywords_minus_generics() {
    // KEYWORDS (the older, coverage-tool-facing set) is unchanged by this
    // module; every non-generic name it already carried must still be
    // covered by the newer, broader BUILTIN_NAMES used for builtin_skip.
    let generic_params = ["T", "U", "K", "V", "E", "R", "S"];
    for name in KEYWORDS {
        if generic_params.contains(name) {
            continue;
        }
        assert!(BUILTIN_NAMES.contains(name), "BUILTIN_NAMES missing {name} from KEYWORDS");
    }
}
