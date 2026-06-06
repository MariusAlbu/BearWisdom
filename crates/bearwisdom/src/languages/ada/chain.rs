// =============================================================================
// ada/chain.rs — Ada chain-walker helpers
//
// Free helpers that probe the symbol index by dotted qname, walk field
// chains across record/struct types, and chase generic-instantiation
// aliases. Used by the Ada `LanguageResolver` impl in `resolve.rs`.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{Resolution, SymbolLookup, RESOLVED_CONFIDENCE};
use crate::types::EdgeKind;

/// Walk a dotted qname looking for any prefix that corresponds to a
/// generic-instantiation symbol (`signature = "instantiates X"`). When
/// found, replace that prefix with the generic's qname so the suffix
/// can resolve against the generic's exported members.
///
/// Example: `String_Vectors.Vector.Append`
///   * `String_Vectors` is a Namespace symbol with
///     `signature = "instantiates Ada.Containers.Vectors"`
///   * Returns `"Ada.Containers.Vectors.Vector.Append"`.
pub(super) fn chase_instantiation(target: &str, lookup: &dyn SymbolLookup) -> Option<String> {
    let parts: Vec<&str> = target.split('.').collect();
    for split in 1..=parts.len() {
        let prefix = parts[..split].join(".");
        let suffix = if split == parts.len() {
            String::new()
        } else {
            format!(".{}", parts[split..].join("."))
        };
        if let Some(sym) = lookup.by_qualified_name(&prefix) {
            if let Some(sig) = &sym.signature {
                if let Some(generic) = sig.strip_prefix("instantiates ") {
                    return Some(format!("{generic}{suffix}"));
                }
            }
        }
        // Also try by simple name if the qname lookup fails — covers
        // bare-name instantiations like `package Foo is new Bar(...)`.
        if split == 1 {
            for sym in lookup.by_name(&prefix) {
                if let Some(sig) = &sym.signature {
                    if let Some(generic) = sig.strip_prefix("instantiates ") {
                        return Some(format!("{generic}{suffix}"));
                    }
                }
            }
        }
    }
    None
}

/// Probe the *package* that owns a type when a member call couldn't be found
/// under the type's own qname.
///
/// Ada subprograms for a type live at package scope, not nested under the
/// type's qname in the index. Given `Pkg.A.B.Type.Method`, the symbol is
/// most likely `Pkg.A.B.Method` — i.e., stripping the penultimate segment
/// (the type name) and probing `members_of("Pkg.A.B")`.
///
/// Returns `None` when the target has fewer than three segments (no package
/// component above the type) or when no match is found.
pub(super) fn probe_package_of_type(
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let parts: Vec<&str> = target.split('.').collect();
    // Need at least: package + type + method (3 segments).
    if parts.len() < 3 {
        return None;
    }
    let method = *parts.last().unwrap();
    let method_lower = method.to_lowercase();
    // Drop the type segment (second-to-last); everything before it is the package.
    let pkg = parts[..parts.len() - 2].join(".");
    for sym in lookup.members_of(&pkg) {
        if sym
            .qualified_name
            .rsplit_once('.')
            .map(|(_, n)| n)
            .unwrap_or(&sym.qualified_name)
            .to_lowercase()
            == method_lower
            && predicates::kind_compatible(edge_kind, &sym.kind)
        {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: RESOLVED_CONFIDENCE,
                strategy: "ada_pkg_of_type",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
    }
    None
}

/// Walk a multi-segment field chain starting from a resolved type qname.
///
/// Given `base_type = "Drivers.Device"` and `segs = ["Port", "Mem_Read"]`,
/// resolves `Port` as a field of `Device`, obtains its type (e.g.,
/// `Drivers.Port_Type`), then probes `members_of` and the package-of-type
/// for `Mem_Read` against that type. Returns the first resolution found.
///
/// Depth is capped at 6 hops to guard against malformed or cyclic indexes.
/// Gives up (returns `None`) if any intermediate field's type is unknown.
pub(super) fn walk_field_chain(
    base_type: &str,
    segs: &[&str],
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    const MAX_DEPTH: usize = 6;
    if segs.len() > MAX_DEPTH {
        return None;
    }
    // segs = [intermediate..., method]. Walk all but the last to resolve types.
    let intermediates = &segs[..segs.len() - 1];
    let method = segs[segs.len() - 1];
    let method_lower = method.to_lowercase();

    let mut current_type = base_type.to_string();
    for field_seg in intermediates {
        let field_lower = field_seg.to_lowercase();
        let field_qname = format!("{current_type}.{field_seg}");
        // Look up the field type by exact qname; fall back to a
        // case-insensitive scan of the current type's members.
        // Ada extractors store field type in `signature = "type: T"` rather
        // than emitting TypeRef edges, so also read the signature as a fallback
        // when field_type_name (which queries TypeRef edges) returns None.
        let next_type = lookup
            .field_type_str(&field_qname)
            .map(|s| s.to_string())
            .or_else(|| {
                lookup.members_of(&current_type).iter().find_map(|m| {
                    let leaf = m
                        .qualified_name
                        .rsplit_once('.')
                        .map(|(_, n)| n)
                        .unwrap_or(&m.qualified_name);
                    if leaf.to_lowercase() != field_lower {
                        return None;
                    }
                    // TypeRef-edge path.
                    lookup
                        .field_type_str(&m.qualified_name)
                        .map(|s| s.to_string())
                        // Signature-path fallback for Ada fields (no TypeRef edges).
                        .or_else(|| {
                            m.signature
                                .as_deref()
                                .and_then(|s| s.strip_prefix("type: "))
                                .map(|t| t.to_string())
                        })
                })
            });
        let Some(next_raw) = next_type else {
            return None; // Chain broken — give up.
        };
        // Expand bare field type to fully-qualified form. When multiple
        // candidates share the same leaf name (e.g. `CFGR_Register` in
        // multiple SVD packages), prefer the one whose package prefix matches
        // the current type's package — otherwise the chain walks into the
        // wrong package.
        let next_leaf = next_raw.split('.').next_back().unwrap_or(&next_raw);
        let pkg_prefix: &str = current_type
            .rsplit_once('.')
            .map(|(p, _)| p)
            .unwrap_or("");
        let expanded = lookup
            .types_by_name(next_leaf)
            .iter()
            .find(|s| !pkg_prefix.is_empty() && s.qualified_name.starts_with(pkg_prefix))
            .or_else(|| lookup.types_by_name(next_leaf).iter().next())
            .map(|s| s.qualified_name.clone())
            .unwrap_or_else(|| next_raw.clone());
        current_type = expanded;
    }

    // current_type is the type reached after all intermediate hops.
    // Probe for the trailing method at type level, then at package level.
    let type_candidate = format!("{current_type}.{method}");
    if let Some(res) = probe_dotted_qname(&type_candidate, edge_kind, lookup) {
        return Some(res);
    }
    let parts: Vec<&str> = current_type.split('.').collect();
    if let Some(pkg_parts) = parts.split_last().map(|(_, rest)| rest) {
        if !pkg_parts.is_empty() {
            let pkg = pkg_parts.join(".");
            for sym in lookup.members_of(&pkg) {
                let sym_leaf = sym
                    .qualified_name
                    .rsplit_once('.')
                    .map(|(_, n)| n)
                    .unwrap_or(&sym.qualified_name);
                if sym_leaf.to_lowercase() == method_lower
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: RESOLVED_CONFIDENCE,
                        strategy: "ada_field_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
    }
    None
}

/// Walk a dotted target back through its parents, probing
/// `members_of(parent)` for a leaf whose name matches case-insensitively
/// and whose kind is compatible with the edge. Returns the first hit.
/// Used by both file-local and cross-file alias substitution.
pub(super) fn probe_dotted_qname(
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let parts: Vec<&str> = target.split('.').collect();
    for split in (1..parts.len()).rev() {
        let parent = parts[..split].join(".");
        let leaf = parts[split..].join(".");
        let leaf_lower = leaf.to_lowercase();
        for sym in lookup.members_of(&parent) {
            let sym_leaf = sym
                .qualified_name
                .rsplit_once('.')
                .map(|(_, n)| n)
                .unwrap_or(&sym.qualified_name)
                .to_lowercase();
            if sym_leaf == leaf_lower
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: RESOLVED_CONFIDENCE,
                    strategy: "ada_alias_substitution",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    None
}
