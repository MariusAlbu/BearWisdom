// =============================================================================
// engine/kinds — the symbol-kind vocabulary the engine classifies by
//
// Extractors record a symbol's kind as a string; the engine reads it in two
// coarse classes. A TYPE kind declares a member set a chain can step off — it
// anchors a namespace path, roots a static access, and is what `types_by_name`
// surfaces. A VALUE kind is a binding whose declared or inferred type roots a
// chain — a local, a parameter, a field. Everything else (functions, modules,
// namespaces) is neither, and each site decides what to do with it.
// =============================================================================

/// `true` when `kind` names a type a `this`/`self` keyword or an inherited
/// member can attach to — a class-like declaration that owns a member set, not
/// a namespace, function, or value.
pub(crate) fn is_type_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "interface"
            | "enum"
            | "trait"
            | "object"
            | "record"
            | "protocol"
            | "actor"
            | "mixin"
            | "annotation"
    )
}

/// `true` when `kind` names a value whose declared type can root a chain.
pub(crate) fn is_value_kind(kind: &str) -> bool {
    matches!(
        kind,
        "variable" | "constant" | "const" | "field" | "property" | "parameter"
    )
}

/// `true` when `kind` names a namespace-like declaration: a container of
/// declarations that is itself neither a type nor a value.
pub(crate) fn is_namespace_kind(kind: &str) -> bool {
    matches!(kind, "namespace" | "module" | "package")
}

#[cfg(test)]
#[path = "kinds_tests.rs"]
mod tests;
