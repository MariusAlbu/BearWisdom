// =============================================================================
// engine/unbound_cause — subdivide a bare-root death by evidence in hand
//
// When neither the ladder nor a chain anchor binds a name, the death still
// carries evidence: an import that binds the name, an enclosing scope that
// declares it as a member, an external index that offers it, or a project
// declaration no rung could reach. Each probe is a hash lookup against state
// the resolver already holds — no rule re-runs, no re-derivation. The probe
// order is most-specific-first, and every death classified here carries a
// cause.
//
// Two classifiers share the evidence probes and differ only in their floor:
// a BARE name that bound nothing is a missing or unreachable declaration; a
// member chain's ROOT that no arm could type is a value expression — a local,
// a parameter, a field, `this` — whose type was never captured.
// =============================================================================

use super::cause::{value_binding_cause, Cause, CauseKind};
use super::contract::{FileContext, SymbolLookup};
use super::kinds::is_value_kind;

/// Classify why the bare name `name` bound nothing. `scope_chain` is the ref
/// site's enclosing scope qnames, innermost first. `file_package_id` scopes the
/// declared-dependency probe to the manifest the source file can see.
pub(super) fn classify_unbound_root(
    name: &str,
    scope_chain: &[String],
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    file_package_id: Option<i64>,
) -> Cause {
    if let Some(cause) = probe_bound_evidence(name, scope_chain, file_ctx, lookup, file_package_id)
    {
        return cause;
    }
    // Declared in the project, unreachable from here. Blame the declaration
    // only when it is unique — an ambiguous name has no single symbol to blame.
    let candidates = lookup.by_name(name);
    if !candidates.is_empty() {
        let blame = if candidates.len() == 1 {
            candidates.first().map(|s| s.id)
        } else {
            None
        };
        return Cause::new(blame, CauseKind::DefinedUnimported);
    }
    Cause::new(None, CauseKind::NameUnknown)
}

/// Classify why a member chain's root segment `name` could not be typed. The
/// evidence probes run first — an import that never linked or an implicit
/// receiver the dispatch missed explains an untyped root as well as a bare
/// name. Past them, the root is a value whose type was never captured: blame
/// the same-file value declaration of that name when exactly one exists, else
/// record the root itself as untyped.
pub(super) fn classify_untyped_root(
    name: &str,
    scope_chain: &[String],
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    file_package_id: Option<i64>,
) -> Cause {
    if let Some(cause) = probe_bound_evidence(name, scope_chain, file_ctx, lookup, file_package_id)
    {
        return cause;
    }
    let candidates = lookup.by_name(name);
    let mut same_file = candidates
        .iter()
        .filter(|s| &*s.file_path == file_ctx.file_path.as_str() && is_value_kind(&s.kind));
    match (same_file.next(), same_file.next()) {
        (Some(binding), None) => value_binding_cause(binding),
        _ => Cause::new(None, CauseKind::UntypedRoot),
    }
}

/// The evidence probes shared by both classifiers, most-specific-first:
/// an import binding the name, an enclosing scope declaring it as a member,
/// or external supply offering it. `None` when no probe applies.
fn probe_bound_evidence(
    name: &str,
    scope_chain: &[String],
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    file_package_id: Option<i64>,
) -> Option<Cause> {
    // A non-wildcard import binds exactly this name: the ladder's import
    // rungs ran and produced nothing, so the import itself never linked.
    if let Some(imp) = file_ctx
        .imports
        .iter()
        .find(|imp| !imp.is_wildcard && imp.bound_name() == name)
    {
        // Manifest refinement: the module IS declared as a dependency for
        // this file's package, yet the specifier links to no indexed file —
        // the declared supply was never materialized. A specifier that does
        // link stays ImportUnlinked: supply exists, the link inside it
        // failed.
        if let Some(spec) = imp.module_path.as_deref() {
            if lookup.is_declared_dependency(file_package_id, spec)
                && lookup
                    .resolve_module_from(&file_ctx.file_path, spec)
                    .is_none()
            {
                return Some(Cause::new(None, CauseKind::ImportDeclaredUnsupplied));
            }
        }
        return Some(Cause::new(None, CauseKind::ImportUnlinked));
    }
    // An enclosing scope declares a member of this name — an implicit-receiver
    // root the dispatch never reached. Scope entries that are not types
    // simply have no members and fall through.
    for scope in scope_chain {
        if let Some(member) = lookup.members_of(scope).iter().find(|s| s.name == name) {
            return Some(Cause::new(Some(member.id), CauseKind::ScopeMemberRoot));
        }
    }
    // Externally attributable: supply exists somewhere (primitive, framework
    // global, or dependency surface) but no external binding materialized.
    if lookup.is_external_name(name, &file_ctx.language) {
        return Some(Cause::new(None, CauseKind::ExternalKnownUnbound));
    }
    None
}

#[cfg(test)]
#[path = "unbound_cause_tests.rs"]
mod tests;
