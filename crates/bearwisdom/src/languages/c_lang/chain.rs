// =============================================================================
// c_lang/chain.rs — C/C++ chain-aware resolution
// =============================================================================
//
// Handles member-access chains like `obj.method()`, `ptr->method()`, and
// `this->field.method()`.
//
// Three-phase algorithm (same as other languages):
//   Phase 1: Determine the root type (SelfRef → enclosing class,
//            Identifier → field_type_name lookup in scope chain).
//   Phase 2: Walk intermediate segments following field/return types.
//   Phase 3: Resolve the final segment on the resolved type.
//
// C++ specifics:
//   - No generic substitution at this stage (template parameters are hard to
//     resolve without a full instantiation graph; covered by the type_alias
//     extraction fix for project-defined typedefs).
//   - Uses `.` as the separator in type_info keys (matching the indexer's
//     convention, even though C++ uses `::` for qualified names).
//   - SelfRef (this->) is handled by finding the enclosing class in the
//     scope chain.
// =============================================================================

use super::predicates::kind_compatible;
use crate::indexer::resolve::engine::{ChainMiss, RefContext, Resolution, SymbolLookup};
use crate::types::{EdgeKind, MemberChain, SegmentKind};
use tracing::debug;

/// Walk a C/C++ MemberChain to resolve the final method/field call.
///
/// Example: `channel->send("hello")` with chain `[channel, send]`:
/// 1. `channel` (Identifier) → look up `field_type_name("EnclosingClass.channel")`
///    → field_type = "SocketChannel"
/// 2. `send` → look up "SocketChannel.send" → resolved!
///
/// For `this->channel->send("hello")` with chain `[this, channel, send]`:
/// 1. `this` (SelfRef) → find enclosing class (e.g., "TcpClientTmpl")
/// 2. `channel` → field_type_name("TcpClientTmpl.channel") → "SocketChannelPtr"
///    which is a typedef → resolved via type_alias lookup
/// 3. `send` → look up "SocketChannel.send" → resolved!
pub(super) fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    // ------------------------------------------------------------------
    // Phase 1: Determine the root type.
    // ------------------------------------------------------------------
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // `this` → find the enclosing class from the scope chain.
            find_enclosing_class(&ref_ctx.scope_chain, lookup)
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known class/struct type? (static access: `ClassName::method()`)
            let is_type = lookup.types_by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "type_alias"
                )
            });
            if is_type {
                Some(name.clone())
            } else {
                // Is it a field/variable on the enclosing class?
                // Try scope chain: e.g., "TcpClientTmpl::connect.channel" → no,
                // but "TcpClientTmpl.channel" → yes (dot-separated in type_info).
                let mut found = None;
                for scope in &ref_ctx.scope_chain {
                    let field_qname = format!("{scope}.{name}");
                    if let Some(type_name) = lookup.field_type_name(&field_qname) {
                        found = Some(type_name.to_string());
                        break;
                    }
                    // Also try with :: scope separator (C++ qualified names can
                    // appear in scope_chain as "hv::TcpClientTmpl" etc.)
                    let field_qname_cc = format!("{}.{name}", scope.replace("::", "."));
                    if let Some(type_name) = lookup.field_type_name(&field_qname_cc) {
                        found = Some(type_name.to_string());
                        break;
                    }
                }
                // Last resort: use the declared_type from the chain segment.
                found.or_else(|| segments[0].declared_type.clone())
            }
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // Normalize `::` to `.` so type_info lookups work (the indexer stores keys
    // with dot separators regardless of source language).
    current_type = normalize_type(&current_type);

    // ------------------------------------------------------------------
    // Phase 2: Walk intermediate segments.
    // ------------------------------------------------------------------
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        // Try field type (e.g., `channel` → "SocketChannel").
        if let Some(next_type) = lookup.field_type_name(&member_qname) {
            current_type = normalize_type(next_type);

            // If the resolved type is a type_alias (typedef), try to
            // dereference it: look up the alias's own field_type.
            // e.g., TSocketChannelPtr → shared_ptr<SocketChannel> is complex,
            // but for simple project typedefs: SocketChannelPtr → SocketChannel.
            current_type = dereference_typedef(&current_type, lookup);
            continue;
        }

        // Try return type (method result in a fluent chain).
        if let Some(raw_return) = lookup.return_type_name(&member_qname) {
            current_type = normalize_type(raw_return);
            current_type = dereference_typedef(&current_type, lookup);
            continue;
        }

        // Members fallback scoped to the resolved type.
        let mut found = false;
        for sym in lookup.members_of(&current_type) {
            if sym.name != seg.name {
                continue;
            }
            if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                current_type = normalize_type(ft);
                current_type = dereference_typedef(&current_type, lookup);
                found = true;
                break;
            }
            if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                current_type = normalize_type(rt);
                current_type = dereference_typedef(&current_type, lookup);
                found = true;
                break;
            }
        }
        if found {
            continue;
        }

        // Lost the chain — record a miss for R4 reload.
        lookup.record_chain_miss(ChainMiss {
            current_type: current_type.clone(),
            target_name: seg.name.clone(),
        });
        return None;
    }

    // ------------------------------------------------------------------
    // Phase 3: Resolve the final segment.
    // ------------------------------------------------------------------
    let last = &segments[segments.len() - 1];
    current_type = dereference_typedef(&current_type, lookup);
    let candidate = format!("{current_type}.{}", last.name);

    // Direct qualified name match.
    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
            debug!(
                strategy = "c_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "c_chain_resolution",
            });
        }
    }

    // Members scoped to the resolved type — unique match wins at 1.0.
    let matches: Vec<_> = lookup
        .members_of(&current_type)
        .iter()
        .filter(|sym| sym.name == last.name && kind_compatible(edge_kind, &sym.kind))
        .cloned()
        .collect();

    match matches.len() {
        0 => {
            lookup.record_chain_miss(ChainMiss {
                current_type: current_type.clone(),
                target_name: last.name.clone(),
            });
            None
        }
        1 => Some(Resolution {
            target_symbol_id: matches[0].id,
            confidence: 1.0,
            strategy: "c_chain_resolution_unique",
        }),
        _ => Some(Resolution {
            target_symbol_id: matches[0].id,
            confidence: 0.95,
            strategy: "c_chain_resolution",
        }),
    }
}

/// Find the enclosing class name from the scope chain.
///
/// C++ scope chains are like `["hv.TcpClientTmpl.connect", "hv.TcpClientTmpl", "hv"]`.
/// We want `"hv.TcpClientTmpl"` — the innermost class/struct.
pub(super) fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct") {
                return Some(scope.clone());
            }
        }
        // Also try normalized form (:: → .)
        let normalized = scope.replace("::", ".");
        if normalized != *scope {
            if let Some(sym) = lookup.by_qualified_name(&normalized) {
                if matches!(sym.kind.as_str(), "class" | "struct") {
                    return Some(normalized);
                }
            }
        }
    }
    // Fallback: penultimate scope is often the class.
    if scope_chain.len() >= 2 {
        return Some(normalize_type(&scope_chain[scope_chain.len() - 2]));
    }
    scope_chain.last().map(|s| normalize_type(s))
}

/// Normalize a C++ qualified name for type_info lookups.
///
/// The indexer stores all keys with `.` as separator. C++ source uses `::`.
/// Convert `std::string` → `std.string`, `hv::TcpClient` → `hv.TcpClient`.
fn normalize_type(name: &str) -> String {
    name.replace("::", ".")
}

/// If `type_name` resolves to a type_alias in the symbol index, try to
/// follow it one hop to the aliased concrete type via field_type_name.
///
/// Example: `TSocketChannelPtr` is a typedef for `shared_ptr<TSocketChannel>`.
/// If indexed as a type_alias with field_type = "SocketChannel", we return
/// "SocketChannel" so Phase 3 can resolve methods on the concrete type.
///
/// This only follows one level — avoids infinite loops and matches the
/// common case (project-defined pointer typedefs).
fn dereference_typedef(type_name: &str, lookup: &dyn SymbolLookup) -> String {
    // Check if type_name is a type_alias symbol.
    for sym in lookup.types_by_name(type_name) {
        if sym.kind == "type_alias" {
            // Look up the typedef's own field_type (populated from its TypeRef
            // by the type_info pass, e.g., TSocketChannelPtr → TSocketChannel).
            if let Some(aliased) = lookup.field_type_name(&sym.qualified_name) {
                return normalize_type(aliased);
            }
        }
    }
    type_name.to_string()
}
