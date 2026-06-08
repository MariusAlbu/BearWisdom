// =============================================================================
// symbol_key.rs — stable symbol identity key (see SYMBOL-IDENTITY.md)
//
// A symbol's key changes IFF its public contract changes, so a key-stable
// survivor across an incremental reparse never invalidates its consumers.
//
// Computed from the extracted symbol alone — syntactic and pre-resolution, so
// it never depends on (and never triggers) name resolution:
//   qualified_name # kind # generic_arity [# "(" param_types ")"]
// Parameter types are the symbol's interned `TypeId`s formatted through the
// workspace `TypeArena` (canonical — whitespace is structural, not source).
// An un-typed parameter formats to the Unknown sentinel and becomes `_`,
// preserving arity so dynamically-typed overloads disambiguate by arity alone.
//
// `mergeable` symbols (namespaces, partial types, reopened classes, merged
// interfaces) take a file-independent key so multi-file declarations collapse
// to one logical id; everything else is scoped by `file_id` so a same-named
// private symbol in two files stays two symbols.
// =============================================================================

use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::{ExtractedSymbol, SymbolKind};

/// True when a symbol can be declared across several files as ONE logical
/// symbol (Roslyn's multi-location symbol). Namespaces and modules spread
/// across files in every language; partial types, reopened classes, and
/// declaration-merged interfaces are per-language.
///
/// NOTE: the per-language set here is the seed from SYMBOL-IDENTITY.md and is
/// expected to grow as languages are validated — it is the one place that
/// decision lives.
pub fn is_mergeable(language: &str, kind: SymbolKind, signature: Option<&str>) -> bool {
    use SymbolKind::*;
    // Universal: a namespace/module is one logical scope no matter how many
    // files contribute declarations to it.
    if matches!(kind, Namespace | Module) {
        return true;
    }
    match language {
        // Ruby classes are always reopenable across files.
        "ruby" => matches!(kind, Class),
        // TypeScript declaration merging: interfaces with the same name merge.
        "typescript" | "tsx" => matches!(kind, Interface),
        // C# partial types are split across files; the `partial` modifier rides
        // in the signature the extractor captured.
        "csharp" => {
            matches!(kind, Class | Struct | Interface)
                && signature.is_some_and(|s| s.contains("partial"))
        }
        _ => false,
    }
}

/// The stable identity key for `sym`. `file_id` scopes non-mergeable symbols;
/// mergeable symbols ignore it so their cross-file declarations share one id.
pub fn symbol_key(
    language: &str,
    sym: &ExtractedSymbol,
    file_id: i64,
    arena: &TypeArena,
) -> String {
    let mut core = String::with_capacity(sym.qualified_name.len() + 16);
    core.push_str(&sym.qualified_name);
    core.push('#');
    core.push_str(sym.kind.as_str());
    core.push('#');
    core.push_str(&sym.generic_params.len().to_string());

    if is_overloadable(sym.kind) {
        core.push_str("#(");
        for (i, &pt) in sym.param_types.iter().enumerate() {
            if i > 0 {
                core.push(',');
            }
            core.push_str(&format_param(arena, pt));
        }
        core.push(')');
    }

    if is_mergeable(language, sym.kind, sym.signature.as_deref()) {
        core
    } else {
        format!("{file_id}:{core}")
    }
}

/// Only callable kinds overload, so only they carry parameter types in the key.
/// For every other kind, name + generic arity is the entire contract.
fn is_overloadable(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Method | SymbolKind::Function | SymbolKind::Constructor
    )
}

/// Canonical spelling of one parameter type. The arena format is structural,
/// so all whitespace is dropped (`List< int >` and `List<int>` are one type);
/// an un-typed parameter (Unknown sentinel or empty) becomes `_` to keep arity.
fn format_param(arena: &TypeArena, ty: TypeId) -> String {
    let formatted = normalize_ws(&arena.format_type(ty));
    if formatted.is_empty() || formatted.eq_ignore_ascii_case("unknown") {
        "_".to_string()
    } else {
        formatted
    }
}

/// Drop every ASCII/Unicode whitespace char — type spellings carry no
/// semantically significant whitespace.
fn normalize_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
#[path = "symbol_key_tests.rs"]
mod tests;
