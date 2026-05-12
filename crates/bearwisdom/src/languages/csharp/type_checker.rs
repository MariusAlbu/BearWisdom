// =============================================================================
// csharp/type_checker.rs — C# type checker
//
// Walks `MemberChain` step-by-step using a C#-specific algorithm: scope
// chain → field type / return type → using-directive qualified-name probe.
// Generic substitution via `TypeEnvironment`. Was `languages/csharp/chain.rs`
// (free function `resolve_via_chain`); ported onto the `TypeChecker` trait
// as part of decision-2026-04-27-e75.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    ChainMiss, FileContext, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::type_env::TypeEnvironment;
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

pub struct CSharpChecker;

impl TypeChecker for CSharpChecker {
    fn language_id(&self) -> &str {
        "csharp"
    }

    fn kind_compatible(&self, edge_kind: EdgeKind, sym_kind: &str) -> bool {
        predicates::kind_compatible(edge_kind, sym_kind)
    }

    /// Walk a MemberChain step-by-step, following field types to resolve
    /// the final segment.
    ///
    /// For `this.repo.FindOne()` with chain `[this, repo, FindOne]`:
    /// 1. `this` → enclosing class (e.g., "MyNs.CatalogService")
    /// 2. `repo` → look up "MyNs.CatalogService.repo" field → "CatalogRepo"
    /// 3. `FindOne` → look up "MyNs.CatalogRepo.FindOne" → resolved!
    ///
    /// Also tries `{using_namespace}.{type}.{method}` for types resolved
    /// via using directives. Generic substitution via `TypeEnvironment`.
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

        // Phase 1: Determine the root type from the first segment.
        let mut initial_generic_args: Vec<String> = Vec::new();
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
                            "class" | "struct" | "interface" | "enum" | "delegate"
                        )
                    });
                    if is_type {
                        Some(name.clone())
                    } else {
                        let mut found = None;
                        for scope in &ref_ctx.scope_chain {
                            let field_qname = format!("{scope}.{name}");
                            if let Some(type_name) = lookup.field_type_name(&field_qname) {
                                initial_generic_args = lookup
                                    .field_type_args(&field_qname)
                                    .unwrap_or(&[])
                                    .to_vec();
                                found = Some(type_name.to_string());
                                break;
                            }
                        }
                        found.or_else(|| segments[0].declared_type.clone())
                    }
                }
            }
            _ => None,
        };

        let mut current_type = root_type?;
        let mut env = TypeEnvironment::new();

        if !initial_generic_args.is_empty() {
            env.enter_generic_context(&current_type, &initial_generic_args, |name| {
                lookup.generic_params(name).map(|p| p.to_vec())
            });
        }

        // Phase 2: Walk intermediate segments.
        for seg in &segments[1..segments.len() - 1] {
            let member_qname = format!("{current_type}.{}", seg.name);

            if let Some(next_type) = lookup.field_type_name(&member_qname) {
                let new_args = lookup
                    .field_type_args(&member_qname)
                    .unwrap_or(&[])
                    .to_vec();
                let resolved_type = env.resolve(next_type);
                env.push_scope();
                if !new_args.is_empty() {
                    env.enter_generic_context(&resolved_type, &new_args, |name| {
                        lookup.generic_params(name).map(|p| p.to_vec())
                    });
                }
                current_type = resolved_type;
                continue;
            }

            if !seg.type_args.is_empty() {
                env.enter_generic_context(&member_qname, &seg.type_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }

            if let Some(raw_return) = lookup.return_type_name(&member_qname) {
                let resolved = env.resolve(raw_return);
                env.push_scope();
                current_type = resolved;
                continue;
            }

            let mut found = false;
            for import in &file_ctx.imports {
                if import.is_wildcard {
                    if let Some(module) = &import.module_path {
                        let qualified_member = format!("{module}.{member_qname}");
                        if let Some(next_type) = lookup.field_type_name(&qualified_member) {
                            let resolved_type = env.resolve(next_type);
                            env.push_scope();
                            current_type = resolved_type;
                            found = true;
                            break;
                        }
                        if let Some(raw_return) = lookup.return_type_name(&qualified_member) {
                            let resolved = env.resolve(raw_return);
                            env.push_scope();
                            current_type = resolved;
                            found = true;
                            break;
                        }
                    }
                }
            }
            if found {
                continue;
            }

            lookup.record_chain_miss(ChainMiss {
                current_type: current_type.clone(),
                target_name: seg.name.clone(),
            });
            return None;
        }

        // Phase 3: Final segment.
        let last = &segments[segments.len() - 1];
        let candidate = format!("{current_type}.{}", last.name);

        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if self.kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "csharp_chain_resolution",
                    resolved_yield_type: csharp_yield_type(sym, &last.type_args, lookup, &mut env),
                    flow_emit: None,
                });
            }
        }

        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(module) = &import.module_path {
                    let ns_candidate = format!("{module}.{candidate}");
                    if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                        if self.kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.95,
                                strategy: "csharp_chain_resolution",
                                resolved_yield_type: csharp_yield_type(
                                    sym, &last.type_args, lookup, &mut env,
                                ),
                                flow_emit: None,
                            });
                        }
                    }
                }
            }
        }

        for sym in lookup.members_of(&current_type) {
            if sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.90,
                    strategy: "csharp_chain_resolution",
                    resolved_yield_type: csharp_yield_type(sym, &last.type_args, lookup, &mut env),
                    flow_emit: None,
                });
            }
        }

        lookup.record_chain_miss(ChainMiss {
            current_type: current_type.clone(),
            target_name: last.name.clone(),
        });
        None
    }
}

/// R5: yield type for a resolved Phase-3 symbol, honoring call-site
/// generic args and the `TypeEnvironment`.
fn csharp_yield_type(
    sym: &SymbolInfo,
    call_site_type_args: &[String],
    lookup: &dyn SymbolLookup,
    env: &mut TypeEnvironment,
) -> Option<String> {
    let raw = lookup
        .return_type_name(&sym.qualified_name)
        .or_else(|| lookup.field_type_name(&sym.qualified_name))?;
    if !call_site_type_args.is_empty() {
        env.enter_generic_context(&sym.qualified_name, call_site_type_args, |name| {
            lookup.generic_params(name).map(|p| p.to_vec())
        });
    }
    Some(env.resolve(raw))
}

/// Find the enclosing class/struct/interface from the scope chain.
fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct" | "interface" | "record") {
                return Some(scope.clone());
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
