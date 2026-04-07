// =============================================================================
// python/primitives.rs — Python primitive types
// =============================================================================

/// Primitive and built-in type names for Python.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Core types
    "int", "float", "str", "bool", "None", "bytes", "list", "dict", "tuple",
    "set", "type", "object", "complex", "frozenset", "memoryview", "range",
    "True", "False",
    // Built-in functions used as types
    "bytearray", "slice", "property", "classmethod", "staticmethod", "super",
    "enumerate", "zip", "map", "filter", "reversed", "sorted",
    // Typing module (PEP 484+)
    "Any", "Union", "Optional", "List", "Dict", "Tuple", "Set", "FrozenSet",
    "Type", "Callable", "Iterator", "Generator", "Coroutine",
    "Awaitable", "AsyncIterator", "AsyncGenerator",
    "Sequence", "MutableSequence", "Mapping", "MutableMapping",
    "Iterable", "Collection", "Hashable", "Sized",
    "ClassVar", "Final", "Literal", "TypeVar", "TypeAlias",
    "Protocol", "runtime_checkable", "overload",
    "TypedDict", "NamedTuple", "NewType",
    "Self", "Never", "NoReturn", "TypeGuard",
    "Annotated", "Concatenate", "ParamSpec", "TypeVarTuple", "Unpack",
    // Exceptions
    "Exception", "BaseException", "ValueError", "TypeError", "KeyError",
    "IndexError", "AttributeError", "RuntimeError", "OSError", "IOError",
    "FileNotFoundError", "PermissionError", "NotImplementedError",
    "StopIteration", "StopAsyncIteration", "GeneratorExit",
    "ImportError", "ModuleNotFoundError", "NameError",
    "AssertionError", "ArithmeticError", "OverflowError", "ZeroDivisionError",
    // ABC / collections.abc
    "ABC", "ABCMeta", "abstractmethod",
    // Dataclasses / attrs
    "dataclass", "field", "Field",
    // Pathlib
    "Path", "PurePath", "PosixPath", "WindowsPath",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
