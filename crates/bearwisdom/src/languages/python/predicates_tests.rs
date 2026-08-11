use super::predicates::is_python_builtin;

#[test]
fn is_python_builtin_drains_bare_call_names() {
    // A bare `len(x)` / `isinstance(x, y)` — no receiver, so this is the
    // shape a `Calls`-edge target_name takes.
    for name in ["len", "str", "isinstance", "dict", "object", "super", "getattr", "print"] {
        assert!(is_python_builtin(name), "{name} should drain as a bare call");
    }
}

#[test]
fn is_python_builtin_drains_bare_type_ref_names() {
    // `x: dict = ...` / `except (ValueError, TypeError):` — the shape a
    // `TypeRef`-edge target_name takes.
    for name in ["dict", "list", "tuple", "set", "frozenset", "Exception", "ValueError", "type"] {
        assert!(is_python_builtin(name), "{name} should drain as a bare type_ref");
    }
}

#[test]
fn is_python_builtin_declines_generic_type_params() {
    // Single-letter TypeVar placeholders bind through GenericParamRule; if
    // builtin_skip claimed them first that rung would never run.
    for name in ["T", "U", "K", "V", "E", "R", "S"] {
        assert!(!is_python_builtin(name), "{name} is a generic param, not a builtin");
    }
}

#[test]
fn is_python_builtin_declines_stdlib_with_source() {
    // typing.Optional / unittest.mock.MagicMock have real `.py` source the
    // cpython-stdlib walker indexes — must resolve through the ladder, not
    // drain as an unknown-project-construct.
    for name in ["Optional", "Union", "MagicMock", "Mock", "dataclass", "wraps"] {
        assert!(!is_python_builtin(name), "{name} has real stdlib source, must not drain");
    }
}

#[test]
fn is_python_builtin_declines_project_names() {
    for name in ["UserDAO", "SupersetException", "compute_metrics", "list_users"] {
        assert!(!is_python_builtin(name), "{name} is a project name, not a builtin");
    }
}
