// =============================================================================
// nuget/type_qname.rs — type identity out of the ECMA-335 type registry.
//
// Two questions the DLL→ParsedFile synthesizer asks about every type it emits:
// which registry entries are real `TypeDef` rows of the assembly being read,
// and what dotted name identifies one of them.
// =============================================================================

use dotscope::metadata::typesystem::{CilType, CilTypeRc};

use super::signature_format::strip_backtick_arity;

/// A nested type nests at most this deep before the walk gives up — a
/// malformed `NestedClass` table can describe a cycle.
const MAX_NESTING_DEPTH: usize = 16;

/// The `TypeDef` table id in the high byte of a metadata token (ECMA-335
/// §II.24.2.6).
const TYPEDEF_TABLE: u8 = 0x02;

/// The assembly's own type definitions, in registry order.
///
/// The type registry holds more than the `TypeDef` table: the type resolver
/// synthesizes an entry per generic instantiation, array, pointer and pinned
/// wrapper it meets in a signature, and those entries copy the flags of the
/// type they wrap — a synthesized generic instantiation is public and sealed
/// exactly like the definition it names, but carries no methods. Restricting
/// to the current assembly's source drops the ones wrapping another
/// assembly's types; the `TypeDef` token check drops the ones wrapping this
/// assembly's own.
pub(super) fn assembly_type_defs(assembly: &dotscope::prelude::CilObject) -> Vec<CilTypeRc> {
    let registry = assembly.types();
    let source = registry.current_assembly_source();
    registry
        .types_from_source(&source)
        .into_iter()
        .filter(|type_def| type_def.token.table() == TYPEDEF_TABLE)
        .collect()
}

/// `(qualified_name, scope_path)` for one type definition, given the arity-
/// stripped name to qualify.
///
/// A nested type's own `namespace` is empty in ECMA-335 — the namespace sits
/// on the outermost enclosing type, and the enclosing chain supplies the
/// segments between it and the type itself. Walking that chain is what turns
/// a bare `Any` into `System.Runtime.InteropServices.JavaScript.JSType.Any`.
pub(super) fn qualified_type_name(
    type_def: &CilType,
    display_name: &str,
) -> (String, Option<String>) {
    let mut namespace = type_def.namespace.clone();
    let mut enclosing: Vec<String> = Vec::new();
    let mut current = type_def.enclosing_type();
    while let Some(outer) = current.filter(|_| enclosing.len() < MAX_NESTING_DEPTH) {
        enclosing.push(strip_backtick_arity(&outer.name).to_string());
        namespace = outer.namespace.clone();
        current = outer.enclosing_type();
    }
    enclosing.reverse();
    join_type_name(&namespace, &enclosing, display_name)
}

/// Joins a namespace, an enclosing-type chain (outermost first) and a type's
/// own name into `(qualified_name, scope_path)`. `scope_path` carries every
/// segment ahead of the name, and is `None` for a type that is neither
/// namespaced nor nested.
fn join_type_name(
    namespace: &str,
    enclosing: &[String],
    display_name: &str,
) -> (String, Option<String>) {
    let mut segments: Vec<&str> = Vec::with_capacity(enclosing.len() + 1);
    if !namespace.is_empty() {
        segments.push(namespace);
    }
    segments.extend(enclosing.iter().map(String::as_str));
    if segments.is_empty() {
        return (display_name.to_string(), None);
    }
    let scope = segments.join(".");
    (format!("{scope}.{display_name}"), Some(scope))
}

#[cfg(test)]
#[path = "type_qname_tests.rs"]
mod tests;
