// TypeScript chain walker — walks `MemberChain` step-by-step using a
// TypeScript-specific algorithm:
//   - Call-root inference for imported callees (`dayjs()`, `expect()`)
//   - Tsconfig-alias rewrites in the call-root probe
//   - npm-globals fallback for jest/vitest-style ambient functions
//   - Declaration merging via `all_by_qualified_name`
//   - Yield-type computation honoring call-site type args
//   - Alias expansion via `expand_alias` (inline types, generics)
//   - Inheritance walk at Phase 2 and Phase 3 (depth-10 cap)

use super::predicates;
use crate::indexer::resolve::engine::{
    intern_yield_type, ChainMiss, FileContext, RefContext, Resolution, SymbolInfo,
    SymbolLookup,
};
use crate::type_checker::alias::expand_alias;
use crate::type_checker::chain::external_type_qname;
use crate::type_checker::type_env::TypeEnvironment;
use crate::types::{EdgeKind, MemberChain, SegmentKind};
use tracing::debug;

/// Resolve a method's raw return-type into the next chain step's type.
///
/// `: this` is the polymorphic-self pattern fluent-API classes use to
/// keep method-chain calls bound to their receiver type.
fn next_chain_type(raw_return: &str, current_type: &str, env: &TypeEnvironment) -> String {
    if raw_return == "this" {
        return current_type.to_string();
    }
    env.resolve(&raw_return)
}

/// Apply alias expansion to `current_type` in-place. When the type is
/// registered as an `AliasTarget::Application`, the expander rewrites
/// `current_type` to the target's head and binds the target's type args
/// to the head's generic params via a fresh scope on `env`. Idempotent
/// for non-aliases.
///
/// `current_args_hint` carries any type args the chain walker just bound
/// for `current_type` (e.g., the args from a `field_type_args` lookup).
fn expand_current_type(
    current_type: &mut String,
    current_args_hint: &[String],
    lookup: &dyn SymbolLookup,
    env: &mut TypeEnvironment,
) {
    let Some((root, args)) = expand_alias(current_type, current_args_hint, lookup, env) else {
        return;
    };
    *current_type = root;
    if !args.is_empty() {
        env.push_scope();
        env.enter_generic_context(current_type, &args, |n| {
            lookup.generic_params(n).map(|p| p.to_vec())
        });
    }
}

/// Walk a MemberChain step-by-step, following field types to resolve
/// the final segment.
///
/// For `this.repo.findOne()` with chain `[this, repo, findOne]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "UserService")
/// 2. `repo` → look up "UserService.repo" field → declared_type = "UserRepo"
/// 3. `findOne` → look up "UserRepo.findOne" in the symbol index → resolved!
///
/// Generic substitution is handled by a `TypeEnvironment`.
pub(crate) fn walk_typescript_chain(
    chain_ref: &MemberChain,
    edge_kind: EdgeKind,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain_ref.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: Determine the root type from the first segment.
    let mut initial_generic_args: Vec<String> = Vec::new();
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            find_enclosing_class(&ref_ctx.scope_chain, lookup)
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            if let Some(local_type) = lookup.local_type(name) {
                Some(local_type)
            } else {
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "class" | "struct" | "interface" | "enum" | "type_alias"
                    )
                });
                if is_type {
                    Some(name.clone())
                } else {
                    let mut found = None;
                    for scope in &ref_ctx.scope_chain {
                        let field_qname = format!("{scope}.{name}");
                        if let Some(type_name) = lookup.field_type_str(&field_qname) {
                            initial_generic_args = lookup
                                .field_type_arg_strs(&field_qname)
                                .unwrap_or_default()
                                .to_vec();
                            found = Some(type_name.to_string());
                            break;
                        }
                    }
                    found
                        .or_else(|| segments[0].declared_type.clone())
                        .or_else(|| resolve_call_root_type(name, file_ctx, ref_ctx, lookup))
                }
            }
        }
        SegmentKind::Construction => {
            // `new X()` chain root — the constructor target is the chain's receiver type.
            let name = &segments[0].name;
            let is_type = lookup.types_by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "type_alias"
                )
            });
            if is_type {
                Some(name.clone())
            } else {
                segments[0].declared_type.clone()
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

    expand_current_type(
        &mut current_type,
        &initial_generic_args,
        lookup,
        &mut env,
    );

    // Phase 2: Walk intermediate segments, following field types or return types.
    for seg in &segments[1..segments.len() - 1] {
        expand_current_type(&mut current_type, &[], lookup, &mut env);
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_str(&member_qname) {
            let new_args = lookup
                .field_type_arg_strs(&member_qname)
                .unwrap_or_default()
                .to_vec();
            let resolved_type = env.resolve(&next_type);
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

        if let Some(raw_return) = lookup.return_type_str(&member_qname) {
            let resolved = next_chain_type(&raw_return, &current_type, &env);
            env.push_scope();
            current_type = resolved;
            continue;
        }

        let mut found = false;
        for sym in lookup.members_of(&current_type) {
            if sym.name != seg.name {
                continue;
            }
            if let Some(ft) = lookup.field_type_str(&sym.qualified_name) {
                let resolved_type = env.resolve(&ft);
                env.push_scope();
                current_type = resolved_type;
                found = true;
                break;
            }
            if let Some(rt) = lookup.return_type_str(&sym.qualified_name) {
                let resolved = next_chain_type(&rt, &current_type, &env);
                env.push_scope();
                current_type = resolved;
                found = true;
                break;
            }
        }
        if found {
            continue;
        }

        if let Some(ext_qname) = external_type_qname(&current_type, lookup) {
            let ext_member = format!("{ext_qname}.{}", seg.name);
            if let Some(next_type) = lookup.field_type_str(&ext_member) {
                let resolved = env.resolve(&next_type);
                env.push_scope();
                current_type = resolved;
                continue;
            }
            if let Some(next_type) = lookup.return_type_str(&ext_member) {
                let resolved = next_chain_type(&next_type, &current_type, &env);
                env.push_scope();
                current_type = resolved;
                continue;
            }
            current_type = ext_qname;
            continue;
        }

        // Inheritance walk: climb parent_class_qname for inherited members.
        // Cap at 10 ancestors to guard against malformed inheritance cycles.
        let mut inherited_resolution: Option<(String, Vec<String>)> = None;
        let mut ancestor = current_type.clone();
        for _ in 0..10 {
            let Some(parent) = lookup.parent_class_qname(&ancestor) else {
                break;
            };
            let parent_owned = parent.to_string();
            let parent_member = format!("{parent_owned}.{}", seg.name);
            if let Some(next_type) = lookup.field_type_str(&parent_member) {
                let new_args = lookup
                    .field_type_arg_strs(&parent_member)
                    .unwrap_or_default()
                    .to_vec();
                inherited_resolution = Some((next_type.to_string(), new_args));
                break;
            }
            if let Some(next_type) = lookup.return_type_str(&parent_member) {
                inherited_resolution = Some((next_type.to_string(), Vec::new()));
                break;
            }
            let mut members_hit: Option<(String, Vec<String>)> = None;
            for sym in lookup.members_of(&parent_owned) {
                if sym.name != seg.name {
                    continue;
                }
                if let Some(ft) = lookup.field_type_str(&sym.qualified_name) {
                    let new_args = lookup
                        .field_type_arg_strs(&sym.qualified_name)
                        .unwrap_or_default()
                        .to_vec();
                    members_hit = Some((ft.to_string(), new_args));
                    break;
                }
                if let Some(rt) = lookup.return_type_str(&sym.qualified_name) {
                    members_hit = Some((rt.to_string(), Vec::new()));
                    break;
                }
            }
            if let Some(hit) = members_hit {
                inherited_resolution = Some(hit);
                break;
            }
            if parent_owned == ancestor {
                break;
            }
            ancestor = parent_owned;
        }
        if let Some((next_type, new_args)) = inherited_resolution {
            let resolved_type = next_chain_type(&next_type, &current_type, &env);
            env.push_scope();
            if !new_args.is_empty() {
                env.enter_generic_context(&resolved_type, &new_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }
            current_type = resolved_type;
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

    // Phase 3: Resolve the final segment on the resolved type.
    let last = &segments[segments.len() - 1];

    expand_current_type(&mut current_type, &[], lookup, &mut env);

    let effective_type = external_type_qname(&current_type, lookup)
        .unwrap_or_else(|| current_type.clone());

    let candidate = format!("{effective_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if predicates::kind_compatible(edge_kind, &sym.kind) {
            debug!(
                strategy = "ts_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %effective_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "ts_chain_resolution",
                resolved_yield_type: intern_yield_type(
                    ts_yield_type(sym, &last.type_args, lookup, &mut env),
                    lookup,
                ),
                flow_emit: None,
            });
        }
    }

    for sym in lookup.members_of(&effective_type) {
        if sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "ts_chain_resolution",
                resolved_yield_type: intern_yield_type(
                    ts_yield_type(sym, &last.type_args, lookup, &mut env),
                    lookup,
                ),
                flow_emit: None,
            });
        }
    }

    // Inheritance walk at Phase 3. Cap at 10 hops.
    let mut ancestor = effective_type.as_str();
    for _ in 0..10 {
        let parent = match lookup.parent_class_qname(ancestor) {
            Some(p) => p,
            None => break,
        };
        let candidate = format!("{parent}.{}", last.name);
        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if predicates::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "ts_chain_inheritance",
                    chain_len = segments.len(),
                    ancestor = %parent,
                    target = %last.name,
                    "resolved via extends chain"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.9,
                    strategy: "ts_chain_inheritance",
                    resolved_yield_type: intern_yield_type(
                        ts_yield_type(sym, &last.type_args, lookup, &mut env),
                        lookup,
                    ),
                    flow_emit: None,
                });
            }
        }
        for sym in lookup.members_of(parent) {
            if sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "ts_chain_inheritance",
                    resolved_yield_type: intern_yield_type(
                        ts_yield_type(sym, &last.type_args, lookup, &mut env),
                        lookup,
                    ),
                    flow_emit: None,
                });
            }
        }
        ancestor = parent;
    }

    lookup.record_chain_miss(ChainMiss {
        current_type: effective_type,
        target_name: last.name.clone(),
    });
    None
}

/// Compute the yield type of a resolved Phase-3 symbol, honoring
/// call-site generic arguments and the active `TypeEnvironment` bindings.
fn ts_yield_type(
    sym: &SymbolInfo,
    call_site_type_args: &[String],
    lookup: &dyn SymbolLookup,
    env: &mut TypeEnvironment,
) -> Option<String> {
    let raw = lookup
        .return_type_str(&sym.qualified_name)
        .or_else(|| lookup.field_type_str(&sym.qualified_name))?;
    if !call_site_type_args.is_empty() {
        env.enter_generic_context(&sym.qualified_name, call_site_type_args, |name| {
            lookup.generic_params(name).map(|p| p.to_vec())
        });
    }
    Some(env.resolve(&raw))
}

/// Resolve the root type when the chain root identifier is a function call.
///
/// For `dayjs().format()` or `expect(x).to.be.equal(y)`, the chain root
/// is an Identifier (the callee name). We need the callee's return_type to
/// seed the chain walk. Strategy:
///   1. Bare-specifier imports: `import dayjs from 'dayjs'`
///   2. Tsconfig alias imports: `import { dayjs } from '@/lib/dayjs'`
///   3. npm globals injection (jest/vitest `globals: true`)
fn resolve_call_root_type(
    name: &str,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for import in &file_ctx.imports {
        if import.imported_name != name && import.alias.as_deref() != Some(name) {
            continue;
        }
        let Some(module) = import.module_path.as_deref() else { continue };
        if module.starts_with('.') || module.starts_with('/') {
            continue;
        }
        let candidate = format!("{module}.{name}");
        if let Some(rt) = lookup.return_type_str(&candidate) {
            return Some(rt.to_string());
        }
        if let Some(ft) = lookup.field_type_str(&candidate) {
            return Some(ft.to_string());
        }
        if let Some(rewritten) = lookup.resolve_path_alias(ref_ctx.file_package_id, module) {
            let alias_candidate = format!("{rewritten}.{name}");
            if let Some(rt) = lookup.return_type_str(&alias_candidate) {
                return Some(rt.to_string());
            }
        }
    }
    let globals_candidate = format!("{}.{name}", crate::ecosystem::npm::NPM_GLOBALS_MODULE);
    if let Some(rt) = lookup.return_type_str(&globals_candidate) {
        return Some(rt.to_string());
    }
    if let Some(ft) = lookup.field_type_str(&globals_candidate) {
        return Some(ft.to_string());
    }
    None
}

/// Find the enclosing class name from the scope chain.
fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    scope_chain.last().cloned()
}
