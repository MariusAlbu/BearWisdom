// =============================================================================
// typescript/type_checker.rs — TypeScript type checker
//
// Concrete `TypeChecker` impl for TypeScript. PR 3 of the type-checker
// consolidation (decision-2026-04-27-e75): collapses the previous
// `typescript/chain.rs` (388 lines) onto this seam. The `TypeScriptResolver`
// dispatches its chain-walking step through `TypeScriptChecker.resolve_chain`
// instead of calling a free function.
//
// The TS-specific chain walker doesn't fit the unified `ChainConfig`-driven
// path the other 12 languages use today — it has bespoke logic for:
//   - Call-root inference for imported callees (`dayjs()`, `expect()`)
//   - Tsconfig-alias rewrites in the call-root probe
//   - npm-globals fallback for jest/vitest-style ambient functions
//   - Declaration merging via `all_by_qualified_name`
//   - Yield-type computation honoring call-site type args
//
// Subsequent PRs add the remaining type-level operations (alias expansion,
// keyof, typeof, mapped, conditional) as additional `TypeChecker` methods,
// mostly delegating to a per-operation module under `type_checker/`.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    ChainMiss, FileContext, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::alias::expand_alias;
use crate::type_checker::chain::external_type_qname;
use crate::type_checker::type_env::TypeEnvironment;
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain, SegmentKind};
use tracing::debug;

/// Apply alias expansion to `current_type` in-place. When the type is
/// registered as an `AliasTarget::Application`, the expander rewrites
/// `current_type` to the target's head and binds the target's type args
/// to the head's generic params via a fresh scope on `env`. Idempotent
/// for non-aliases — returns immediately without touching `env`.
///
/// `current_args_hint` carries any type args the chain walker just bound
/// for `current_type` (e.g., the args from a `field_type_args` lookup).
/// `expand_alias` uses these to substitute the alias's declared params
/// when resolving the target's args.
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

/// TypeScript type checker. Unit struct — owns no state; constructed on
/// demand by `TypeScriptPlugin::type_checker()` and held in the engine's
/// checker registry.
pub struct TypeScriptChecker;

impl TypeChecker for TypeScriptChecker {
    fn language_id(&self) -> &str {
        "typescript"
    }

    fn kind_compatible(&self, edge_kind: EdgeKind, sym_kind: &str) -> bool {
        // Delegates to the per-language predicate fn. The fn predates the
        // TypeChecker seam and is still consumed by JS-only paths that
        // don't go through this trait yet, so it stays public in the
        // predicates module.
        predicates::kind_compatible(edge_kind, sym_kind)
    }

    /// Walk a MemberChain step-by-step, following field types to resolve
    /// the final segment.
    ///
    /// For `this.repo.findOne()` with chain `[this, repo, findOne]`:
    /// 1. `this` → find enclosing class from scope_chain (e.g., "UserService")
    /// 2. `repo` → look up "UserService.repo" field → declared_type = "UserRepo"
    /// 3. `findOne` → look up "UserRepo.findOne" in the symbol index → resolved!
    ///
    /// Generic substitution is handled by a `TypeEnvironment`: when we
    /// encounter a field like `repo: Repository<User>`, we bind the
    /// Repository's type params to concrete args in a new scope. When the
    /// resolved return type is a bound param (e.g., "T"), `env.resolve("T")`
    /// returns the concrete type ("User").
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
            // Single-segment chains (e.g., `foo()`) are handled by the regular scope chain.
            return None;
        }

        // Phase 1: Determine the root type from the first segment.
        let mut initial_generic_args: Vec<String> = Vec::new();
        let root_type = match segments[0].kind {
            SegmentKind::SelfRef => {
                // `this` → find the enclosing class from the scope chain.
                find_enclosing_class(&ref_ctx.scope_chain, lookup)
            }
            SegmentKind::Identifier => {
                let name = &segments[0].name;

                // R5: per-file flow inference takes precedence over global lookups.
                // A local-variable shadow correctly hides a same-named class.
                if let Some(local_type) = lookup.local_type(name) {
                    Some(local_type)
                } else {
                    // Is it a known class/type? (static access: `ClassName.method()`)
                    let is_type = lookup.types_by_name(name).iter().any(|s| {
                        matches!(
                            s.kind.as_str(),
                            "class" | "struct" | "interface" | "enum" | "type_alias"
                        )
                    });
                    if is_type {
                        Some(name.clone())
                    } else {
                        // Is it a field on the enclosing class?
                        let mut found = None;
                        for scope in &ref_ctx.scope_chain {
                            let field_qname = format!("{scope}.{name}");
                            if let Some(type_name) = lookup.field_type_name(&field_qname) {
                                // Capture generic args from the field declaration.
                                initial_generic_args = lookup
                                    .field_type_args(&field_qname)
                                    .unwrap_or(&[])
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
            _ => None,
        };

        let mut current_type = root_type?;

        // Build a TypeEnvironment for this chain walk.
        // When entering a generic type (e.g., Repository<User>), we push a scope binding
        // T=User. When a return type is a bound param, env.resolve() substitutes it.
        let mut env = TypeEnvironment::new();

        // If the root was a generic field, enter its generic context now.
        if !initial_generic_args.is_empty() {
            env.enter_generic_context(&current_type, &initial_generic_args, |name| {
                lookup.generic_params(name).map(|p| p.to_vec())
            });
        }

        // Alias expansion at the root: if the resolved root type is itself
        // a type alias (e.g., `type UserMap = Map<string, User>`), unwrap it
        // before Phase 2's field/method lookups — those would otherwise
        // attempt `UserMap.member` and find nothing.
        expand_current_type(
            &mut current_type,
            &initial_generic_args,
            lookup,
            &mut env,
        );

        // Phase 2: Walk intermediate segments, following field types or return types.
        for seg in &segments[1..segments.len() - 1] {
            // Each iteration may have just inherited `current_type` from a
            // field_type / return_type continuation in the previous body —
            // unwrap any alias before computing the member qname so the
            // lookups target the underlying type, not the alias name.
            expand_current_type(&mut current_type, &[], lookup, &mut env);
            let member_qname = format!("{current_type}.{}", seg.name);

            // Try field type (property access).
            if let Some(next_type) = lookup.field_type_name(&member_qname) {
                let new_args = lookup
                    .field_type_args(&member_qname)
                    .unwrap_or(&[])
                    .to_vec();
                // Resolve the new type through the environment (handles T → User etc).
                let resolved_type = env.resolve(next_type);
                // Transition to the new type's generic context.
                env.push_scope();
                if !new_args.is_empty() {
                    env.enter_generic_context(&resolved_type, &new_args, |name| {
                        lookup.generic_params(name).map(|p| p.to_vec())
                    });
                }
                current_type = resolved_type;
                continue;
            }

            // R5: call-site type args (`findOne<User>()`) bind the method's own
            // generic parameters before its return type resolves.
            if !seg.type_args.is_empty() {
                env.enter_generic_context(&member_qname, &seg.type_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }

            // Try return type (method call result in a fluent chain).
            if let Some(raw_return) = lookup.return_type_name(&member_qname) {
                // Use TypeEnvironment to substitute type params (e.g., "T" → "User").
                let resolved = env.resolve(raw_return);
                // Clear current generic bindings and enter context for the new type.
                env.push_scope();
                current_type = resolved;
                continue;
            }

            // Members fallback scoped to the resolved type.
            let mut found = false;
            for sym in lookup.members_of(&current_type) {
                if sym.name != seg.name {
                    continue;
                }
                if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                    let resolved_type = env.resolve(ft);
                    env.push_scope();
                    current_type = resolved_type;
                    found = true;
                    break;
                }
                if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                    let resolved = env.resolve(rt);
                    env.push_scope();
                    current_type = resolved;
                    found = true;
                    break;
                }
            }
            if found {
                continue;
            }

            // External-type fallback: `current_type` may be a short name like
            // "Assertion" whose external symbol lives as "chai.Assertion".
            // Resolve to the full external qname and retry member lookups.
            if let Some(ext_qname) = external_type_qname(&current_type, lookup) {
                let ext_member = format!("{ext_qname}.{}", seg.name);
                if let Some(next_type) = lookup.field_type_name(&ext_member) {
                    let resolved = env.resolve(next_type);
                    env.push_scope();
                    current_type = resolved;
                    continue;
                }
                if let Some(next_type) = lookup.return_type_name(&ext_member) {
                    let resolved = env.resolve(next_type);
                    env.push_scope();
                    current_type = resolved;
                    continue;
                }
                // The external type is known but this member isn't typed — keep
                // walking with the full external qname so Phase 3 can still match.
                current_type = ext_qname;
                continue;
            }

            // Lost the chain — can't determine the next type. Record a miss
            // so the R3 reload pass can pull current_type's definition file.
            // Upgrade short names ("Assertion") to full external qnames
            // ("chai.Assertion") when an external type owns the short name —
            // the reload pass needs the leading dep segment to address the
            // right ExternalDepRoot.
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

        // Final alias expansion before the leaf lookup. Covers two cases the
        // Phase-2 loop misses:
        //   - Chains of length 2 (`x.foo` style) where the loop body never
        //     runs, so the post-Phase-1 expansion is the only one that fired.
        //     If the previous iter ended with `current_type` updated to an
        //     alias, this dereferences it before the leaf lookup.
        //   - Chains where the last intermediate hop landed on an alias name
        //     (e.g., `x.subscriptions.first()` where `subscriptions` is typed
        //     as a `SubscriptionList = Subscription[]`).
        expand_current_type(&mut current_type, &[], lookup, &mut env);

        // Resolve `current_type` to its external qname if it's a short name
        // (e.g., "Assertion" -> "chai.Assertion") so the final lookups hit the
        // package-prefixed symbol rather than returning None.
        let effective_type = external_type_qname(&current_type, lookup)
            .unwrap_or_else(|| current_type.clone());

        let candidate = format!("{effective_type}.{}", last.name);

        // Direct qualified name match.
        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if self.kind_compatible(edge_kind, &sym.kind) {
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
                    resolved_yield_type: ts_yield_type(sym, &last.type_args, lookup, &mut env),
                });
            }
        }

        // Members scoped to the resolved type.
        for sym in lookup.members_of(&effective_type) {
            if sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "ts_chain_resolution",
                    resolved_yield_type: ts_yield_type(sym, &last.type_args, lookup, &mut env),
                });
            }
        }

        // Inheritance walk: if the method isn't on `effective_type` directly,
        // follow the `extends` chain. `BehaviorSubject extends Subject<T>` and
        // `Subject` is where `asObservable` lives — class-member-only lookup
        // would stop at BehaviorSubject and miss every inherited method. Cap
        // at 10 hops to guard against malformed cycles.
        let mut ancestor = effective_type.as_str();
        for _ in 0..10 {
            let parent = match lookup.parent_class_qname(ancestor) {
                Some(p) => p,
                None => break,
            };
            let candidate = format!("{parent}.{}", last.name);
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.kind_compatible(edge_kind, &sym.kind) {
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
                        resolved_yield_type: ts_yield_type(sym, &last.type_args, lookup, &mut env),
                    });
                }
            }
            for sym in lookup.members_of(parent) {
                if sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.85,
                        strategy: "ts_chain_inheritance",
                        resolved_yield_type: ts_yield_type(sym, &last.type_args, lookup, &mut env),
                    });
                }
            }
            ancestor = parent;
        }

        // Final-segment miss: the walker landed on `effective_type` but no
        // member named `last.name` is indexed under it or any ancestor. Same
        // R3 reload signal as the intermediate-segment bail-out — feed
        // `effective_type` to resolve_symbol so its definition file gets pulled.
        lookup.record_chain_miss(ChainMiss {
            current_type: effective_type,
            target_name: last.name.clone(),
        });
        None
    }
}

// ---------------------------------------------------------------------------
// Internal helpers — same logic that lived in typescript/chain.rs before PR 3.
// ---------------------------------------------------------------------------

/// R5: compute the yield type of a resolved Phase-3 symbol, honoring
/// call-site generic arguments (`findOne<User>()`) and the active
/// `TypeEnvironment` bindings accumulated during chain walking.
fn ts_yield_type(
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

/// Resolve the root type when the chain root identifier is a function call.
///
/// For `dayjs().format()` or `expect(x).to.be.equal(y)`, the chain root
/// is an Identifier (the callee name). We need the callee's return_type to
/// seed the chain walk. Strategy:
///   1. Bare-specifier imports: `import dayjs from 'dayjs'` → probe
///      `dayjs.dayjs` return_type_name (the synthetic declares it as
///      returning `dayjs.Dayjs`). Also covers `import { vi } from 'vitest'`.
///   2. Tsconfig alias imports: `import { dayjs } from '@/lib/dayjs'` →
///      rewrite the alias, probe the resolved file's re-export. This handles
///      the common workspace pattern of `packages/dayjs/index.ts` wrapping
///      the real npm package.
///   3. npm globals: `declare global { … }` in @types packages exposes
///      bare callee names (jest's `expect`, vitest globals) under the
///      `__npm_globals__` sentinel.
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
        // Pass 1: bare specifier (`import dayjs from 'dayjs'`).
        let candidate = format!("{module}.{name}");
        if let Some(rt) = lookup.return_type_name(&candidate) {
            return Some(rt.to_string());
        }
        if let Some(ft) = lookup.field_type_name(&candidate) {
            return Some(ft.to_string());
        }
        // Pass 2: tsconfig alias rewrite (`@/lib/dayjs` → `apps/web/src/lib/dayjs`).
        if let Some(rewritten) = lookup.resolve_tsconfig_alias(ref_ctx.file_package_id, module) {
            let alias_candidate = format!("{rewritten}.{name}");
            if let Some(rt) = lookup.return_type_name(&alias_candidate) {
                return Some(rt.to_string());
            }
        }
    }
    // Pass 3: npm globals injection (jest/vitest `globals: true`).
    let globals_candidate = format!("{}.{name}", crate::ecosystem::npm::NPM_GLOBALS_MODULE);
    if let Some(rt) = lookup.return_type_name(&globals_candidate) {
        return Some(rt.to_string());
    }
    if let Some(ft) = lookup.field_type_name(&globals_candidate) {
        return Some(ft.to_string());
    }
    None
}

/// Find the enclosing class name from the scope chain.
/// scope_chain is `["UserService.findAll", "UserService"]` — we want "UserService".
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
    // Fallback: the shortest scope entry is often the class.
    scope_chain.last().cloned()
}
