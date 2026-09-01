// =============================================================================
// engine/root_import_discipline — an import-bound chain root types through its
// import, or dies with the import as cause
//
// When a file explicitly imports the name a chain roots on, the root's type
// must come from the import's own candidate set: the imported module's ext
// files, the internally-linked module file, or the workspace package the
// specifier names. The unscoped by-name fallbacks are forbidden for such a
// root — a same-named symbol from an unrelated file is a hijack that types
// the receiver wrongly and turns every downstream member step into a
// misleading miss.
//
// Denial requires positive evidence the import SHOULD have linked: a
// scheme-prefixed specifier (`node:assert/strict` — never an internal module
// path in any indexed language) or an externally-attested head. A specifier
// this module cannot positively judge leaves the root unconstrained, so
// module systems whose linking is not wired here keep their existing arms.
// =============================================================================

use crate::type_checker::core::types::TypeArena;

use super::cause::{Cause, CauseKind};
use super::chain::{import_scoped_external_root, Receiver};
use super::contract::{FileContext, ImportEntry, Symbol, SymbolLookup};
use super::support::{is_bare_module_specifier, is_type_kind, workspace_sub_path};

pub(super) enum RootImportOutcome {
    /// No non-wildcard import binds this name, or the import cannot be
    /// positively judged — the caller's fallback arms run unchanged.
    Unconstrained,
    /// A candidate from the import's own scope typed the root.
    Typed(Receiver),
    /// The import is the root's cause: unlinked, or its candidate's own
    /// type was never captured.
    Deny(Cause),
}

pub(super) fn apply(
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    seg: &crate::types::ChainSegment,
) -> RootImportOutcome {
    let Some(entry) = binding_import(file_ctx, &seg.name) else {
        return RootImportOutcome::Unconstrained;
    };
    let Some(spec) = entry.module_path.as_deref() else {
        return RootImportOutcome::Unconstrained;
    };
    // The module's files declare the ORIGINAL name of a rename import.
    let lookup_name = if entry.alias.as_deref() == Some(seg.name.as_str()) {
        entry.imported_name.as_str()
    } else {
        seg.name.as_str()
    };

    // External candidate set (ext files under the imported module).
    if let Some(recv) = import_scoped_external_root(file_ctx, lookup, arena, seg) {
        return RootImportOutcome::Typed(recv);
    }

    // Internally-linked module (relative paths, path aliases, language module
    // resolvers): candidates are the linked file's own symbols.
    let internal = lookup.in_module_from(&file_ctx.file_path, spec);
    if let Some(sym) = internal.iter().find(|s| s.name == lookup_name) {
        return type_candidate(lookup, arena, sym, seg.is_call);
    }
    if spec.starts_with('.') {
        // Relative specifier with no internal link recovered here: linking
        // coverage varies by lookup, so this is not evidence of a dead import.
        return RootImportOutcome::Unconstrained;
    }
    if lookup.resolve_module_from(&file_ctx.file_path, spec).is_some() {
        // The module links to a file but that file does not declare the name
        // (re-export shapes this lookup cannot see). Not judgeable.
        return RootImportOutcome::Unconstrained;
    }
    if !is_bare_module_specifier(spec) {
        return RootImportOutcome::Unconstrained;
    }

    // Workspace package: candidates are the package's own symbols, narrowed
    // by the deep-import sub-path when one is present. `workspace_package_id`
    // peels deep specifiers itself.
    if let Some(pkg_id) = lookup.workspace_package_id(spec) {
        let sub_path = workspace_sub_path(spec, lookup);
        let candidates = lookup.symbols_in_package(pkg_id);
        let sub = sub_path.as_deref();
        let hit = candidates
            .iter()
            .filter(|s| s.name == lookup_name)
            .find(|s| sub.is_none_or(|sub| s.file_path.contains(sub)))
            .or_else(|| candidates.iter().find(|s| s.name == lookup_name));
        return match hit {
            Some(sym) => type_candidate(lookup, arena, sym, seg.is_call),
            None => RootImportOutcome::Deny(Cause::new(None, CauseKind::ImportUnlinked)),
        };
    }

    // Nothing links the specifier. Deny only on positive external evidence:
    // a scheme prefix, or a head the external index attests to. Anything
    // else stays unconstrained — absence of linking is not proof of death
    // for module systems this gate cannot see.
    if has_scheme_prefix(spec) || attested_external(lookup, spec, &file_ctx.language) {
        return RootImportOutcome::Deny(Cause::new(None, CauseKind::ImportUnlinked));
    }
    RootImportOutcome::Unconstrained
}

/// The non-wildcard import that binds `name` at the use site — by its
/// imported name or its alias, mirroring the external candidate arm's match.
fn binding_import<'a>(file_ctx: &'a FileContext, name: &str) -> Option<&'a ImportEntry> {
    file_ctx
        .imports
        .iter()
        .filter(|imp| !imp.is_wildcard)
        .find(|imp| imp.imported_name == name || imp.alias.as_deref() == Some(name))
}

/// Type a scoped candidate, or blame it: a candidate whose own return/field
/// type was never captured is the root's cause, with the candidate's id.
fn type_candidate(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    sym: &Symbol,
    is_call: bool,
) -> RootImportOutcome {
    if is_type_kind(&sym.kind) {
        return RootImportOutcome::Typed(Receiver::new(
            super::head_decl::nominal_head(lookup, arena, sym),
            sym.id,
        ));
    }
    if is_call {
        if let Some(id) = super::type_slots::return_type_by_identity(lookup, sym).or_else(|| {
            lookup
                .return_type_str(&sym.qualified_name)
                .map(|s| arena.intern_type_str(&s))
        }) {
            return RootImportOutcome::Typed(Receiver::untyped(id));
        }
        return RootImportOutcome::Deny(Cause::new(Some(sym.id), CauseKind::UncapturedReturn));
    }
    if let Some(id) = super::type_slots::field_type_by_identity(lookup, sym).or_else(|| {
        lookup
            .field_type_str(&sym.qualified_name)
            .map(|s| arena.intern_type_str(&s))
    }) {
        return RootImportOutcome::Typed(Receiver::untyped(id));
    }
    RootImportOutcome::Deny(Cause::new(Some(sym.id), CauseKind::UncapturedField))
}

/// A single-colon URI-style scheme prefix (`node:fs`, `sass:math`). A double
/// colon is a qualified-path separator, never a scheme.
fn has_scheme_prefix(spec: &str) -> bool {
    super::module_scheme::strip_scheme_prefix(spec).is_some()
}

/// The external index attests to the specifier or one of its leading path
/// prefixes (`@scope/pkg/sub` → `@scope/pkg` → `@scope`).
fn attested_external(lookup: &dyn SymbolLookup, spec: &str, language: &str) -> bool {
    if lookup.is_external_name(spec, language) {
        return true;
    }
    let segments: Vec<&str> = spec.split('/').collect();
    (1..segments.len().min(3))
        .rev()
        .any(|k| lookup.is_external_name(&segments[..k].join("/"), language))
}

#[cfg(test)]
#[path = "root_import_discipline_tests.rs"]
mod tests;
