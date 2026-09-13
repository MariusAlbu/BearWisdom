// =============================================================================
// engine/root_declaration_space — which declaration space owns a bare root
//
// One name can be declared twice: once in the type space, once in the value
// space. A bare chain root is a RECEIVER, so the value declaration owns it
// unless the type declaration is evaluable too — a class's statics and a
// namespace's exports are reached through the type's own name, a shape-only
// declaration's members are not.
//
// Two external declarations of a name belong to ONE declaration group when
// they sit in one external package: a package's ambient declarations share a
// scope across its declaration files, so a global's value face
// (`const D: DConstructor`) and its shape face (`interface D`) are the same
// name even when they are written in different files of that package. A path
// no ecosystem adapter claims exposes no package grammar to read, so its file
// is the widest scope that can be proven — a same-named value elsewhere stays
// a stranger.
// =============================================================================

use crate::ecosystem::package_specifier::external_package_key_from_path;

use super::contract::{Symbol, SymbolSet};
use super::kinds::is_shape_only_kind;

/// The virtual-path prefix every external (dependency-sourced) file carries.
const EXTERNAL_PATH_PREFIX: &str = "ext:";

/// `true` when an EXTERNAL value declaration must yield a bare chain root to a
/// same-named type declaration.
///
/// A bare-name pick of an external value is the weakest evidence tier: no
/// scope qualification, no ambient registration, and the import-scoped
/// external root that owns genuine import attribution declined upstream. Such
/// a pick yields in three shapes:
///   - the project itself declares the type — the head is a static-access /
///     construction root on the project's own type, not a value borrowed from
///     an unrelated dependency's surface;
///   - the value is a MEMBER, which never roots a bare name;
///   - the value is a stranger: every same-named type declaration it could
///     pair with is either evaluable in its own right or sits outside the
///     value's declaration group.
///
/// The value keeps the root when its group declares the name's shape and
/// nothing evaluable — the static surface of such a pair lives on the value's
/// declared type (`D.now` is on the constructor object, never on the instance
/// shape). Scope-qualified hits, same-file values and internal imports never
/// reach here, so genuine value shadowing keeps winning on its own evidence.
pub(super) fn value_yields_to_type(value: &Symbol, types_named: &SymbolSet<'_>) -> bool {
    if !is_external(&value.file_path) || types_named.is_empty() {
        return false;
    }
    if types_named.iter().any(|t| !is_external(&t.file_path)) || is_external_member(value) {
        return true;
    }
    !types_named.iter().any(|decl| merges_with(decl, value))
}

/// `true` when an external declaration is a MEMBER — a foreign type's
/// property/field, a function's parameter. A member needs a receiver and a
/// parameter a scope, so neither ever roots a bare name. Without this, the
/// blanket external ownership that ambient globals rely on lets a lib type's
/// same-named property type an untyped local.
pub(super) fn is_external_member(sym: &Symbol) -> bool {
    is_external(&sym.file_path) && matches!(sym.kind.as_str(), "field" | "property" | "parameter")
}

/// `true` when `decl` and `value` are two faces of one name rather than two
/// names that happen to be spelled alike. One declaration file proves it
/// outright. Across files of one external package it holds for a shape-only
/// `decl`: the pair's evaluable surface is then the value's declared type, so
/// nothing is lost by rooting on the value. A `decl` that is evaluable in its
/// own right keeps the root instead, which is what an unrelated same-named
/// value in that package would otherwise take.
fn merges_with(decl: &Symbol, value: &Symbol) -> bool {
    decl.file_path == value.file_path
        || (is_shape_only_kind(&decl.kind)
            && same_external_package(&decl.file_path, &value.file_path))
}

/// `true` when both paths resolve to one external package key. A path whose
/// grammar no ecosystem adapter owns yields no key and never matches.
fn same_external_package(left: &str, right: &str) -> bool {
    match (
        external_package_key_from_path(left),
        external_package_key_from_path(right),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn is_external(path: &str) -> bool {
    path.starts_with(EXTERNAL_PATH_PREFIX)
}

#[cfg(test)]
#[path = "root_declaration_space_tests.rs"]
mod tests;
