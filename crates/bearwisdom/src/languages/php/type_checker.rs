// =============================================================================
// php/type_checker.rs — PHP type checker
//
// Walks `MemberChain` step-by-step using a PHP-specific algorithm. Includes
// inheritance walking for Eloquent Model subclasses (`__callStatic`-forwarded
// Builder methods). Was `languages/php/chain.rs`; ported per
// decision-2026-04-27-e75.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    ChainMiss, FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::chain::{external_type_qname, simple_yield_type};
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

pub struct PhpChecker;

impl TypeChecker for PhpChecker {
    fn language_id(&self) -> &str {
        "php"
    }

    fn kind_compatible(&self, edge_kind: EdgeKind, sym_kind: &str) -> bool {
        predicates::kind_compatible(edge_kind, sym_kind)
    }

    fn resolve_chain(
        &self,
        chain_ref: &MemberChain,
        edge_kind: EdgeKind,
        file_ctx: Option<&FileContext>,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let file_ctx = file_ctx?;
        let segments = &chain_ref.segments;
        if segments.len() < 2 {
            return None;
        }

        // Phase 1: root type.
        let root_type = match segments[0].kind {
            SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
            SegmentKind::Identifier => {
                let name = &segments[0].name;

                if let Some(local_type) = lookup.local_type(name) {
                    Some(local_type)
                } else {
                    let is_type = lookup.types_by_name(name).iter().any(|s| {
                        matches!(
                            s.kind.as_str(),
                            "class" | "interface" | "enum" | "type_alias"
                        )
                    });
                    if is_type {
                        Some(name.clone())
                    } else {
                        let mut found = None;
                        for scope in &ref_ctx.scope_chain {
                            let field_qname = format!("{scope}.{name}");
                            if let Some(type_name) = lookup.field_type_name(&field_qname) {
                                found = Some(type_name.to_string());
                                break;
                            }
                        }
                        found.or_else(|| segments[0].declared_type.clone())
                    }
                }
            }
            // PHP static call: `ClassName::method()` — segment is TypeAccess.
            SegmentKind::TypeAccess => {
                let name = &segments[0].name;
                let qualified = lookup
                    .types_by_name(name)
                    .iter()
                    .find(|s| {
                        matches!(s.kind.as_str(), "class" | "interface" | "enum" | "type_alias")
                    })
                    .map(|s| s.qualified_name.clone())
                    .unwrap_or_else(|| name.clone());
                Some(qualified)
            }
            _ => None,
        };

        let mut current_type = root_type?;

        // Phase 2: intermediate segments.
        for seg in &segments[1..segments.len() - 1] {
            let member_qname = format!("{current_type}.{}", seg.name);

            if let Some(next_type) = lookup.field_type_name(&member_qname) {
                current_type = next_type.to_string();
                continue;
            }
            if let Some(next_type) = lookup.return_type_name(&member_qname) {
                current_type = next_type.to_string();
                continue;
            }

            let mut found = false;
            for import in &file_ctx.imports {
                if let Some(module) = &import.module_path {
                    let qualified_member = format!("{module}.{member_qname}");
                    if let Some(next_type) = lookup.field_type_name(&qualified_member) {
                        current_type = next_type.to_string();
                        found = true;
                        break;
                    }
                    if let Some(next_type) = lookup.return_type_name(&qualified_member) {
                        current_type = next_type.to_string();
                        found = true;
                        break;
                    }
                }
            }
            if found {
                continue;
            }

            if let Some(ext_qname) = external_type_qname(&current_type, lookup) {
                let ext_member = format!("{ext_qname}.{}", seg.name);
                if let Some(next_type) = lookup.field_type_name(&ext_member) {
                    current_type = next_type.to_string();
                    continue;
                }
                if let Some(next_type) = lookup.return_type_name(&ext_member) {
                    current_type = next_type.to_string();
                    continue;
                }
                current_type = ext_qname;
                continue;
            }

            let miss_type = external_type_qname(&current_type, lookup)
                .unwrap_or_else(|| current_type.clone());
            lookup.record_chain_miss(ChainMiss {
                current_type: miss_type,
                target_name: seg.name.clone(),
            });
            return None;
        }

        // Phase 3: final segment.
        let last = &segments[segments.len() - 1];
        let effective_type = external_type_qname(&current_type, lookup)
            .unwrap_or_else(|| current_type.clone());
        let candidate = format!("{effective_type}.{}", last.name);

        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if self.kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "php_chain_resolution",
                    resolved_yield_type: simple_yield_type(sym, lookup),
                    flow_emit: None,
                });
            }
        }

        for import in &file_ctx.imports {
            if let Some(module) = &import.module_path {
                let ns_candidate = format!("{module}.{candidate}");
                if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                    if self.kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "php_chain_resolution",
                            resolved_yield_type: simple_yield_type(sym, lookup),
                            flow_emit: None,
                        });
                    }
                }
            }
        }

        for sym in lookup.members_of(&effective_type) {
            if sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.90,
                    strategy: "php_chain_resolution",
                    resolved_yield_type: simple_yield_type(sym, lookup),
                    flow_emit: None,
                });
            }
        }

        // Inheritance walk for Eloquent-style `__callStatic` forwarding.
        let mut cls = effective_type.as_str();
        for _ in 0..10 {
            match lookup.parent_class_qname(cls) {
                None => break,
                Some(parent) => {
                    let parent_candidate = format!("{parent}.{}", last.name);
                    if let Some(sym) = lookup.by_qualified_name(&parent_candidate) {
                        if self.kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.85,
                                strategy: "php_chain_inherited",
                                resolved_yield_type: simple_yield_type(sym, lookup),
                                flow_emit: None,
                            });
                        }
                    }
                    for sym in lookup.members_of(parent) {
                        if sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.80,
                                strategy: "php_chain_inherited",
                                resolved_yield_type: simple_yield_type(sym, lookup),
                                flow_emit: None,
                            });
                        }
                    }
                    cls = parent;
                }
            }
        }

        lookup.record_chain_miss(ChainMiss {
            current_type: effective_type,
            target_name: last.name.clone(),
        });
        None
    }
}

/// Find the enclosing class/interface from the scope chain.
fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
