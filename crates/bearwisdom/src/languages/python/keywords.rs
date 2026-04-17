// =============================================================================
// python/keywords.rs — Python primitive types
// =============================================================================

/// Primitive and built-in type names for Python.
/// typing/Pathlib/ABC/dataclasses symbols are NOT listed here — they come
/// from the CpythonStdlib ecosystem as indexed external symbols. Only core
/// builtin types (implemented in C, not in the stdlib source tree) and
/// built-in functions not reliably indexed remain.
pub(crate) const KEYWORDS: &[&str] = &[
    // Core types — C-implemented builtins, not in cpython source tree
    "int", "float", "str", "bool", "None", "bytes", "list", "dict", "tuple",
    "set", "type", "object", "complex", "frozenset", "memoryview", "range",
    "True", "False",
    // Built-in functions — C-implemented, not reliably indexed
    "bytearray", "classmethod", "staticmethod", "super",
    "zip", "sorted",
    // Exceptions — C-implemented in cpython, not in indexed stdlib source
    "Exception", "BaseException", "ValueError", "TypeError", "KeyError",
    "IndexError", "AttributeError", "RuntimeError", "OSError", "IOError",
    "FileNotFoundError", "PermissionError", "NotImplementedError",
    "StopIteration", "StopAsyncIteration", "GeneratorExit",
    "ImportError", "ModuleNotFoundError", "NameError",
    "AssertionError", "ArithmeticError", "OverflowError", "ZeroDivisionError",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
