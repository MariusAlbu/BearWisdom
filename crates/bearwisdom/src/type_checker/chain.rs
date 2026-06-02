// =============================================================================
// type_checker/chain.rs — Unified chain-aware resolution
//
// Replaces 8 per-language chain.rs files with a single parameterized
// implementation. Language differences are captured in `ChainConfig`,
// not duplicated code.
//
// Three-phase algorithm:
//   Phase 1: Determine root type (SelfRef → enclosing class, Identifier → field type)
//   Phase 2: Walk intermediate segments following field_type_name / return_type_name
//   Phase 3: Resolve final segment on the resolved type
//
// PR 2 of the type checker consolidation (decision-2026-04-27-e75): moved
// here from `indexer/resolve/chain_walker.rs`. The `TypeChecker` trait gains
// a default `resolve_chain` method that delegates to `resolve_via_chain`;
// per-language checkers can override in subsequent PRs.
// =============================================================================

use crate::indexer::resolve::engine::{
    intern_yield_type, ChainMiss, FileContext, RefContext, Resolution, SymbolInfo,
    SymbolLookup,
};
use crate::type_checker::alias::expand_alias;
use crate::type_checker::type_env::TypeEnvironment;
use crate::types::{EdgeKind, MemberChain, SegmentKind};
use tracing::debug;

// ---------------------------------------------------------------------------
// ChainConfig — captures all language-specific variation
// ---------------------------------------------------------------------------

/// Language-specific configuration for chain resolution.
/// All differences between the per-language chain walkers are captured here.
pub struct ChainConfig {
    /// Strategy prefix for diagnostics (e.g., "ts", "python", "rust").
    pub strategy_prefix: &'static str,

    /// Normalize a type name before lookup.
    /// Rust: replace `::` with `.`. All others: identity.
    pub normalize_type: fn(&str) -> String,

    /// Whether the language has a self/this reference (SelfRef segments).
    pub has_self_ref: bool,

    /// Symbol kinds that count as "enclosing type" for SelfRef resolution.
    /// e.g., `&["class", "struct", "interface"]` for TypeScript.
    pub enclosing_type_kinds: &'static [&'static str],

    /// Symbol kinds that count as "static type" for root Identifier checks.
    /// e.g., `&["class", "struct", "interface", "enum", "type_alias"]` for TypeScript.
    pub static_type_kinds: &'static [&'static str],

    /// Whether to use TypeEnvironment for generic type substitution.
    /// true for TypeScript, Go, C#, Java, Kotlin, Scala, Dart, Swift.
    pub use_generics: bool,

    /// Whether to try namespace-qualified lookups via file imports.
    /// Used by Java (wildcard only), C# (wildcard + generics), PHP (all imports).
    pub namespace_lookup: NamespaceLookup,

    /// Edge-kind / symbol-kind compatibility check.
    pub kind_compatible: fn(EdgeKind, &str) -> bool,

    /// Optional capabilities a language opts into. `ChainExtensions::NONE`
    /// keeps the bare three-phase walk; languages whose source forks carried
    /// extra hops (TypeScript) flip the relevant flags.
    pub extensions: ChainExtensions,
}

/// Opt-in chain-walk capabilities. Each flag adds a fallback hop that fires
/// only after the bare field/return/member lookups miss, so enabling one can
/// only widen resolution — never change a hit the base walk already produced.
pub struct ChainExtensions {
    /// Expand `current_type` through `expand_alias` at the root and before
    /// each member lookup, so a value typed as a type-alias name walks to the
    /// alias's concrete head (`type UserMap = Map<string, User>` → `Map`).
    pub expand_aliases: bool,

    /// On a member miss, climb `parent_class_qname` (depth-10) retrying the
    /// member on each ancestor — inherited fields/methods reachable by the
    /// `extends` chain.
    pub walk_inheritance: bool,

    /// Promote a short `current_type` to its external-package qname via
    /// `external_type_qname` before member lookup, so chains landing on an
    /// externally-declared type (`Assertion` → `chai.Assertion`) find members
    /// keyed under the package-prefixed qname.
    pub promote_external_qname: bool,

    /// Accept a `Construction` root segment (`new X().m()`): the constructed
    /// type is the chain's receiver.
    pub root_construction: bool,

    /// As a final-segment last resort, bind `receiver.Method()` to a static
    /// extension method `static R Method(this Receiver x, ...)` declared in a
    /// static class. Match is by member name plus a `this <current_type>` first
    /// parameter read from the signature, and visibility-blind (no import-scope
    /// gate). C#-specific: the static-method-with-`this`-receiver shape and the
    /// global, using-blind search both diverge from package-scoped static
    /// imports in Java/Kotlin.
    pub extension_method_fallback: bool,

    /// Last-resort root resolver for a bare-identifier call root the import
    /// scan missed — carries the per-ecosystem ambient-globals probe (e.g.
    /// jest/vitest `globals: true`). Returns the root type name.
    pub root_fallback: Option<fn(&str, &dyn SymbolLookup) -> Option<String>>,

    /// Accept a `TypeAccess` root segment (`ClassName::method()`): the root
    /// resolves to the named type's qualified name (falling back to the bare
    /// name when no type-kind symbol owns it). The receiver is then that type.
    pub root_type_access: bool,

}

impl ChainExtensions {
    /// No extensions — the bare three-phase walk. The default for every
    /// language whose chain rules fit field/return/member lookups directly.
    pub const NONE: ChainExtensions = ChainExtensions {
        expand_aliases: false,
        walk_inheritance: false,
        promote_external_qname: false,
        root_construction: false,
        extension_method_fallback: false,
        root_fallback: None,
        root_type_access: false,
    };
}

/// How to handle namespace-aware chain resolution.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NamespaceLookup {
    /// No namespace lookup (TS, Python, Go, Ruby, Rust).
    None,
    /// Try wildcard imports only (Java).
    WildcardOnly,
    /// Try all imports (PHP).
    AllImports,
    /// Try wildcard imports with generic resolution (C#).
    WildcardWithGenerics,
}

/// Identity type normalizer (no-op) — used by most languages.
pub fn identity_normalize(s: &str) -> String {
    s.to_string()
}

// ---------------------------------------------------------------------------
// Unified chain resolver
// ---------------------------------------------------------------------------

/// Walk a MemberChain step-by-step, following field/return types to resolve
/// the final segment.
///
/// For `this.repo.findOne()` with chain `[this, repo, findOne]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "UserService")
/// 2. `repo` → look up "UserService.repo" field → field_type = "Repository<User>"
/// 3. `findOne` → look up "Repository.findOne" → resolved!
///
/// Generic substitution (when `config.use_generics`): a `TypeEnvironment`
/// tracks bindings like T=User. When a return type is "T", it resolves to "User".
pub fn resolve_via_chain(
    config: &ChainConfig,
    chain: &MemberChain,
    edge_kind: EdgeKind,
    file_ctx: Option<&FileContext>,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    let strategy = config.strategy_prefix;

    // ------------------------------------------------------------------
    // Phase 1: Determine the root type from the first segment.
    // ------------------------------------------------------------------
    let mut initial_generic_args: Vec<String> = Vec::new();

    let root_type = match segments[0].kind {
        SegmentKind::SelfRef if config.has_self_ref => {
            find_enclosing_type(&ref_ctx.scope_chain, lookup, config.enclosing_type_kinds)
                .map(|t| (config.normalize_type)(&t))
        }
        SegmentKind::TypeAccess if config.extensions.root_type_access => {
            // `ClassName::method()` — the static-access root names a type.
            // Resolve to that type's qualified name so members key under the
            // namespaced qname; fall back to the bare name when unindexed.
            let name = &segments[0].name;
            let qualified = lookup
                .types_by_name(name)
                .iter()
                .find(|s| config.static_type_kinds.iter().any(|&k| s.kind == k))
                .map(|s| s.qualified_name.clone())
                .unwrap_or_else(|| name.clone());
            Some((config.normalize_type)(&qualified))
        }
        SegmentKind::Construction if config.extensions.root_construction => {
            // `new X().m()` — the constructed type is the receiver. Accept it
            // when it names a known type; otherwise fall back to the segment's
            // declared type (synthetic constructor roots carry one).
            let name = &segments[0].name;
            let is_type = lookup.types_by_name(name).iter().any(|s| {
                config.static_type_kinds.iter().any(|&k| s.kind == k)
            });
            if is_type {
                Some((config.normalize_type)(name))
            } else {
                segments[0]
                    .declared_type
                    .as_ref()
                    .map(|t| (config.normalize_type)(t))
            }
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // R5: per-file flow inference takes precedence over global lookups.
            // A local-variable shadow correctly hides a same-named class —
            // `let x = foo(); x.bar()` resolves `x` to `foo`'s return type
            // before the resolver checks globals or fields.
            if let Some(local_type) = lookup.local_type(name) {
                Some((config.normalize_type)(&local_type))
            } else {
                // Static type access: `ClassName.method()` or `EnumType.Variant`.
                // Use types_by_name (pre-filtered to type-kind symbols) instead of
                // by_name — with externals indexed, common names like "Error" or
                // "Context" collect tens of thousands of non-type candidates that
                // .any() would scan in the worst case.
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    config
                        .static_type_kinds
                        .iter()
                        .any(|&k| s.kind == k)
                });
                if is_type {
                    Some((config.normalize_type)(name))
                } else {
                    // Field on enclosing class: `this.repo` where `repo` is a field.
                    let mut found = None;
                    for scope in &ref_ctx.scope_chain {
                        let field_qname = format!("{scope}.{name}");
                        if let Some(type_name) = lookup.field_type_str(&field_qname) {
                            if config.use_generics {
                                initial_generic_args = lookup
                                    .field_type_arg_strs(&field_qname)
                                    .unwrap_or_default();
                            }
                            found = Some((config.normalize_type)(&type_name));
                            break;
                        }
                    }
                    // Declared type is a parse-time annotation — always the
                    // strongest hint when available. Consult imports as the
                    // final fallback: `import { vi } from 'vitest'; vi.spyOn(...)`
                    // where the external origin declares a field_type on the
                    // imported name (`export const vi: Vi`), or the import
                    // itself *is* a type (`import { Button } from 'fake-ui'`).
                    found
                        .or_else(|| {
                            segments[0]
                                .declared_type
                                .as_ref()
                                .map(|t| (config.normalize_type)(t))
                        })
                        .or_else(|| resolve_import_root_type(name, file_ctx, ref_ctx, config, lookup))
                        .or_else(|| {
                            config.extensions.root_fallback.and_then(|f| f(name, lookup))
                        })
                }
            }
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // ------------------------------------------------------------------
    // Phase 2: Walk intermediate segments.
    // ------------------------------------------------------------------

    // Optional TypeEnvironment for generic substitution.
    let mut env = if config.use_generics {
        let mut e = TypeEnvironment::new();
        if !initial_generic_args.is_empty() {
            e.enter_generic_context(&current_type, &initial_generic_args, |name| {
                lookup.generic_params(name).map(|p| p.to_vec())
            });
        }
        Some(e)
    } else {
        None
    };

    // Alias-aware root: a value typed as a type-alias name has no members of
    // its own — walk to the alias's concrete head first.
    expand_current_type(config, &mut current_type, &initial_generic_args, lookup, env.as_mut());

    for seg in &segments[1..segments.len() - 1] {
        // Re-expand: the previous yield may itself name an alias.
        expand_current_type(config, &mut current_type, &[], lookup, env.as_mut());
        let member_qname = format!("{current_type}.{}", seg.name);

        // Try field type (property access).
        if let Some(next_type) = lookup.field_type_str(&member_qname) {
            let resolved = resolve_and_enter_generics(
                &next_type,
                &member_qname,
                config,
                lookup,
                env.as_mut(),
                true,
            );
            current_type = resolved;
            continue;
        }

        // R5: call-site type arguments (`repo.findOne<User>()`) bind the
        // method's own generic parameters before its return type resolves.
        // Without this, `findOne<User>()` returning `T` would walk the chain
        // as `T` rather than `User`. Only meaningful for languages whose
        // `ChainConfig::use_generics` is set.
        if config.use_generics && !seg.type_args.is_empty() {
            if let Some(e) = env.as_mut() {
                e.enter_generic_context(&member_qname, &seg.type_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }
        }

        // Try return type (method call result in a fluent chain).
        if let Some(raw_return) = lookup.return_type_str(&member_qname) {
            let resolved = resolve_and_enter_generics(
                &raw_return,
                &member_qname,
                config,
                lookup,
                env.as_mut(),
                false,
            );
            current_type = resolved;
            continue;
        }

        // Namespace-qualified fallback (Java, C#, PHP).
        if config.namespace_lookup != NamespaceLookup::None {
            if let Some(file_ctx) = file_ctx {
                if let Some(next) =
                    resolve_via_namespace(config, file_ctx, &member_qname, lookup, env.as_mut())
                {
                    current_type = next;
                    continue;
                }
            }
        }

        // Members fallback: find the segment among direct children of current_type.
        // Using members_of avoids the O(total-symbols-named-seg.name) fan-out that
        // by_name produces once external ecosystems are indexed.
        let mut found = false;
        for sym in lookup.members_of(&current_type) {
            if sym.name != seg.name {
                continue;
            }
            if let Some(ft) = lookup.field_type_str(&sym.qualified_name) {
                let resolved = resolve_and_enter_generics(
                    &ft,
                    &sym.qualified_name,
                    config,
                    lookup,
                    env.as_mut(),
                    true,
                );
                current_type = resolved;
                found = true;
                break;
            }
            if let Some(rt) = lookup.return_type_str(&sym.qualified_name) {
                let resolved = resolve_and_enter_generics(
                    &rt,
                    &sym.qualified_name,
                    config,
                    lookup,
                    env.as_mut(),
                    false,
                );
                current_type = resolved;
                found = true;
                break;
            }
        }
        if found {
            continue;
        }

        // External-qname promotion: a short `current_type` shadowed by an
        // external library type ("Assertion" → "chai.Assertion") keys its
        // members under the package-prefixed qname. Retry the member there.
        if config.extensions.promote_external_qname {
            if let Some(ext_qname) = external_type_qname(&current_type, lookup) {
                let ext_member = format!("{ext_qname}.{}", seg.name);
                if let Some(ft) = lookup.field_type_str(&ext_member) {
                    current_type = resolve_and_enter_generics(
                        &ft, &ext_member, config, lookup, env.as_mut(), true,
                    );
                    continue;
                }
                if let Some(rt) = lookup.return_type_str(&ext_member) {
                    current_type = resolve_and_enter_generics(
                        &rt, &ext_member, config, lookup, env.as_mut(), false,
                    );
                    continue;
                }
                // Type known, member not — advance to the external type so the
                // remaining segments resolve against its qname.
                current_type = ext_qname;
                continue;
            }
        }

        // Inheritance walk: climb `parent_class_qname` retrying the member on
        // each ancestor (depth-10, cycle-guarded). Covers inherited
        // fields/methods reachable through the `extends` chain.
        if config.extensions.walk_inheritance {
            if let Some(next) =
                walk_inheritance_for_member(&current_type, &seg.name, config, lookup, env.as_mut())
            {
                current_type = next;
                continue;
            }
        }

        // Lost the chain — can't determine the next type. Record the miss
        // for R3 lazy reload: a second pass will call resolve_symbol on
        // `current_type`'s owning ecosystem dep to pull its definition file.
        let miss_type = if config.extensions.promote_external_qname {
            external_type_qname(&current_type, lookup).unwrap_or_else(|| current_type.clone())
        } else {
            current_type.clone()
        };
        lookup.record_chain_miss(ChainMiss {
            current_type: miss_type,
            target_name: seg.name.clone(),
            module: None,
        });
        return None;
    }

    // ------------------------------------------------------------------
    // Phase 3: Resolve the final segment on the resolved type.
    // ------------------------------------------------------------------
    let last = &segments[segments.len() - 1];

    // Alias-aware final hop, then promote a short receiver to its external
    // package qname. With extensions off, `effective_type == current_type`.
    expand_current_type(config, &mut current_type, &[], lookup, env.as_mut());
    let effective_type = if config.extensions.promote_external_qname {
        external_type_qname(&current_type, lookup).unwrap_or_else(|| current_type.clone())
    } else {
        current_type.clone()
    };

    let candidate = format!("{effective_type}.{}", last.name);

    // Direct qualified name match.
    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if (config.kind_compatible)(edge_kind, &sym.kind) {
            debug!(
                strategy = %format!("{strategy}_chain_resolution"),
                chain_len = segments.len(),
                resolved_type = %effective_type,
                target = %last.name,
                "resolved"
            );
            let yield_type = compute_yield_type(
                sym, &last.type_args, config, lookup, env.as_mut(),
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: chain_strategy(strategy),
                resolved_yield_type: intern_yield_type(yield_type, lookup),
                flow_emit: None,
            });
        }
    }

    // Namespace-qualified final resolution (Java, C#, PHP).
    if config.namespace_lookup != NamespaceLookup::None {
        if let Some(file_ctx) = file_ctx {
            if let Some(res) = resolve_final_via_namespace(
                config, file_ctx, &effective_type, &last.name, edge_kind, lookup,
            ) {
                return Some(res);
            }
        }
    }

    // by_name scoped to the resolved type.
    //
    // Constrain the prefix match: `effective_type = "Foo"` must be followed by
    // a `.` so it doesn't spuriously collide with `FooBar.bar`. Then collect
    // all matches — a single match is deterministic enough to emit at
    // confidence 1.0 via a dedicated "*_chain_resolution_unique" strategy,
    // while multiple matches keep the 0.95 hedge.
    let type_prefix = format!("{effective_type}.");
    let matches: Vec<&SymbolInfo> = lookup
        .by_name(&last.name)
        .iter()
        .filter(|sym| {
            (sym.qualified_name == effective_type
                || sym.qualified_name.starts_with(&type_prefix))
                && (config.kind_compatible)(edge_kind, &sym.kind)
        })
        .collect();
    match matches.len() {
        0 => {}
        1 => {
            let yield_type = compute_yield_type(
                matches[0], &last.type_args, config, lookup, env.as_mut(),
            );
            return Some(Resolution {
                target_symbol_id: matches[0].id,
                confidence: 1.0,
                strategy: chain_strategy_unique(strategy),
                resolved_yield_type: intern_yield_type(yield_type, lookup),
                flow_emit: None,
            });
        }
        _ => {
            // Ambiguous: multiple candidates share this (type_prefix, name,
            // compatible_kind) triple. The previous behavior was to pick
            // `matches[0]` at 0.95 confidence, but `matches[0]` is hash-
            // seed-dependent — whichever symbol happened to land first in
            // `by_name` wins. That's both non-deterministic AND wrong
            // often enough to corrupt dead-code detection. Fall through to
            // the heuristic tier, which applies a `0.50 / sqrt(n)` decay
            // and is intentionally honest about ambiguity.
        }
    }

    // Members-of final fallback: a direct child of `effective_type` whose
    // simple name matches, emitted at 0.95. The by_name-prefix block above
    // already covers the deterministic single-hit case; this catches the
    // member declared directly under the type but not surfaced as a unique
    // by_name match (e.g. an external type whose members share common names).
    if config.extensions.walk_inheritance {
        for sym in lookup.members_of(&effective_type) {
            if sym.name == last.name && (config.kind_compatible)(edge_kind, &sym.kind) {
                let yield_type = compute_yield_type(
                    sym, &last.type_args, config, lookup, env.as_mut(),
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: chain_strategy(strategy),
                    resolved_yield_type: intern_yield_type(yield_type, lookup),
                    flow_emit: None,
                });
            }
        }

        // Inheritance walk: climb `parent_class_qname` (depth-10) retrying the
        // final member on each ancestor. by_qualified_name hits at 0.9,
        // members_of hits at 0.85.
        let mut ancestor = effective_type.as_str();
        for _ in 0..10 {
            let parent = match lookup.parent_class_qname(ancestor) {
                Some(p) => p,
                None => break,
            };
            let cand = format!("{parent}.{}", last.name);
            if let Some(sym) = lookup.by_qualified_name(&cand) {
                if (config.kind_compatible)(edge_kind, &sym.kind) {
                    let yield_type = compute_yield_type(
                        sym, &last.type_args, config, lookup, env.as_mut(),
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: chain_strategy_inheritance(strategy),
                        resolved_yield_type: intern_yield_type(yield_type, lookup),
                        flow_emit: None,
                    });
                }
            }
            for sym in lookup.members_of(parent) {
                if sym.name == last.name && (config.kind_compatible)(edge_kind, &sym.kind) {
                    let yield_type = compute_yield_type(
                        sym, &last.type_args, config, lookup, env.as_mut(),
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.85,
                        strategy: chain_strategy_inheritance(strategy),
                        resolved_yield_type: intern_yield_type(yield_type, lookup),
                        flow_emit: None,
                    });
                }
            }
            ancestor = parent;
        }
    }

    // Extension-method last resort: `receiver.Method()` binds to a static
    // method `static R Method(this Receiver x, ...)` in a static class. Scan
    // `by_name(last.name)` for a method/function whose signature carries a
    // `this <effective_type>` first parameter. Fires after every receiver-typed
    // lookup misses, so it only widens resolution.
    if config.extensions.extension_method_fallback {
        for sym in lookup.by_name(&last.name) {
            if (sym.kind == "method" || sym.kind == "function")
                && (config.kind_compatible)(edge_kind, &sym.kind)
                && signature_is_extension_on(sym.signature.as_deref(), &effective_type)
            {
                let yield_type =
                    compute_yield_type(sym, &last.type_args, config, lookup, env.as_mut());
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: chain_strategy_extension(strategy),
                    resolved_yield_type: intern_yield_type(yield_type, lookup),
                    flow_emit: None,
                });
            }
        }
    }

    // Final-segment miss: walked to effective_type but no `.last.name` found
    // anywhere under it. Same R3 reload signal as the intermediate-segment
    // bail-out above.
    lookup.record_chain_miss(ChainMiss {
        current_type: effective_type,
        target_name: last.name.clone(),
        module: None,
    });
    None
}

/// True when `sig` is a C# extension-method signature whose `this`-qualified
/// first parameter has receiver type `receiver_type` (compared by simple
/// name). An extension method is a static method whose first parameter is
/// `this ReceiverType name`; the receiver token is the word after `this `.
/// Returns false on a malformed/absent signature.
fn signature_is_extension_on(sig: Option<&str>, receiver_type: &str) -> bool {
    let Some(sig) = sig else { return false };
    let Some(open) = sig.find('(') else { return false };
    let Some(after_this) = sig[open + 1..].trim_start().strip_prefix("this ") else {
        return false;
    };
    let recv = after_this
        .trim_start()
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ')' | '<' | '['))
        .next()
        .unwrap_or("");
    let recv_simple = recv.rsplit('.').next().unwrap_or(recv);
    let want_simple = receiver_type.rsplit('.').next().unwrap_or(receiver_type);
    !recv_simple.is_empty() && recv_simple == want_simple
}

/// R5: compute the yield type of a resolved symbol for per-language
/// chain walkers that don't use a `TypeEnvironment`.
///
/// Used by Python, Ruby, Rust, C, Go per-language chain walkers. Returns
/// the method's return type or the field's declared type as a String, or
/// `None` when neither is recorded. Callers typically pass this straight
/// into `Resolution::resolved_yield_type`.
pub fn simple_yield_type(sym: &SymbolInfo, lookup: &dyn SymbolLookup) -> Option<String> {
    lookup
        .return_type_name(&sym.qualified_name)
        .or_else(|| lookup.field_type_name(&sym.qualified_name))
        .map(|s| s.to_string())
}

/// R5: compute the type the resolved final-segment symbol *yields* for
/// forward flow inference.
///
/// For a method, that's the return type; for a field/property, it's the
/// declared field type. The type is normalized and then substituted through
/// the active `TypeEnvironment` so call-site generics and outer class
/// generics both apply (`repo.findOne<User>()` returning `T` yields `User`).
///
/// Returns `None` when the target has no recorded return/field type. In
/// that case the resolver simply can't record a local-type binding — the
/// next chain walker for the same local falls back to existing logic.
fn compute_yield_type(
    sym: &SymbolInfo,
    call_site_type_args: &[String],
    config: &ChainConfig,
    lookup: &dyn SymbolLookup,
    env: Option<&mut TypeEnvironment>,
) -> Option<String> {
    // Methods: return type. Fields/properties: field type. Priority is
    // return_type because a `()` at the site already tells us it's a call.
    let raw = lookup
        .return_type_name(&sym.qualified_name)
        .or_else(|| lookup.field_type_name(&sym.qualified_name));
    let raw = raw?;
    let normalized = (config.normalize_type)(raw);
    match env {
        Some(e) => {
            // Bind the final segment's own call-site type args (if any) so a
            // method that returns `T` yields the caller's concrete type.
            if config.use_generics && !call_site_type_args.is_empty() {
                e.enter_generic_context(&sym.qualified_name, call_site_type_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }
            Some(e.resolve(&normalized))
        }
        None => Some(normalized),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve a short type name to its fully-qualified external symbol name.
///
/// When the chain walker has `current_type = "Assertion"` (just the short name,
/// as returned by a function's return_type stored in TypeInfo), but the external
/// symbol lives under `chai.Assertion`, the direct lookup `field_type_name("Assertion.to")`
/// fails. This helper bridges the gap:
///
/// 1. Look up `by_name(current_type)`.
/// 2. Filter for symbols whose file_path starts with `"ext:"` (external origin).
/// 3. Return the first match's `qualified_name` (e.g., `"chai.Assertion"`).
///
/// The caller then retries member lookups using the full qname:
///   `field_type_name("chai.Assertion.to")`  →  success.
///
/// Returns `None` when no external symbol owns this short name, preserving
/// the existing bail-out behaviour.
pub fn external_type_qname(current_type: &str, lookup: &dyn SymbolLookup) -> Option<String> {
    // types_by_name is the pre-filtered type-kind subset — the external-qname
    // fallback only cares about type-like symbols anyway, and the smaller
    // candidate pool keeps this fast even when externals collide on common
    // type names ("Builder", "Context", "Request", ...).
    lookup
        .types_by_name(current_type)
        .iter()
        .find(|s| s.file_path.starts_with("ext:"))
        .map(|s| s.qualified_name.clone())
}

/// Root-type resolution for an imported identifier.
///
/// Covers the `import { vi } from 'vitest'; vi.spyOn(...)` shape where the
/// chain's root is a value brought into scope by a bare-specifier import. Two
/// paths:
///   1. The external origin declares a field_type on the name
///      (`export const vi: Vi` → `vitest.vi` has field_type `Vi` → root is
///      `vitest.Vi`).
///   2. The import itself IS the type (`import { Button } from 'fake-ui';
///      new Button(...)`) — use the qualified name directly so the chain
///      walker's Phase 2/3 can reach instance members.
///
/// Relative imports are skipped — the resolver's same-language import handling
/// covers those and modelling cross-file flow here duplicates work.
fn resolve_import_root_type(
    name: &str,
    file_ctx: Option<&FileContext>,
    ref_ctx: &RefContext,
    config: &ChainConfig,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    let fc = file_ctx?;
    for import in &fc.imports {
        if import.imported_name.as_str() != name
            && import.alias.as_deref() != Some(name)
        {
            continue;
        }
        let Some(module) = import.module_path.as_deref() else { continue };
        if module.starts_with('.') || module.starts_with('/') {
            continue;
        }
        let candidate = format!("{module}.{name}");
        // Call-root form (`dayjs()` / `expect(x)`): the callee's return type
        // seeds the chain. Probe return type first so `dayjs().format()`
        // resolves against the call result, then the value's declared field
        // type, then the import-is-a-type case.
        if let Some(rt) = lookup.return_type_str(&candidate) {
            return Some((config.normalize_type)(&rt));
        }
        if let Some(ty) = lookup.field_type_str(&candidate) {
            return Some((config.normalize_type)(&ty));
        }
        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if config.static_type_kinds.iter().any(|&k| k == sym.kind) {
                return Some((config.normalize_type)(&candidate));
            }
        }
        // tsconfig `paths` alias: the specifier may be an alias (`@/lib/dayjs`)
        // that rewrites to a real package path before the qname matches.
        if let Some(rewritten) = lookup.resolve_path_alias(ref_ctx.file_package_id, module) {
            let alias_candidate = format!("{rewritten}.{name}");
            if let Some(rt) = lookup.return_type_str(&alias_candidate) {
                return Some((config.normalize_type)(&rt));
            }
            if let Some(ft) = lookup.field_type_str(&alias_candidate) {
                return Some((config.normalize_type)(&ft));
            }
        }
    }
    None
}

/// Find the enclosing type from the scope chain, matching against
/// the specified set of type kinds.
pub fn find_enclosing_type(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
    type_kinds: &[&str],
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if type_kinds.iter().any(|&k| sym.kind == k) {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: penultimate scope is often the class (method → class → package).
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}

/// Expand `current_type` through `expand_alias` when the language opts into
/// alias-aware walking. A value typed as a type-alias name (`type UserMap =
/// Map<string, User>`) has no members of its own — rewrite it to the alias's
/// concrete head and bind the alias's type args into a fresh `env` scope so
/// deeper segments substitute consistently. No-op when extensions are off or
/// the type isn't an alias.
///
/// When `env` is `None` (non-generic languages like C), a throwaway
/// `TypeEnvironment` serves the alias lookup — arg bindings are discarded
/// since the language has no generics to substitute. This allows no-args
/// typedefs (`type Foo = Bar`) to collapse even without `use_generics`.
fn expand_current_type(
    config: &ChainConfig,
    current_type: &mut String,
    current_args_hint: &[String],
    lookup: &dyn SymbolLookup,
    env: Option<&mut TypeEnvironment>,
) {
    if !config.extensions.expand_aliases {
        return;
    }
    match env {
        Some(env) => {
            let Some((root, args)) = expand_alias(current_type, current_args_hint, lookup, env)
            else {
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
        None => {
            // Non-generic language: use a throwaway env — arg bindings are
            // discarded, but the root rewrite still fires for no-args aliases.
            let mut throwaway = TypeEnvironment::new();
            if let Some((root, _)) =
                expand_alias(current_type, current_args_hint, lookup, &mut throwaway)
            {
                *current_type = root;
            }
        }
    }
}


/// Climb `parent_class_qname` from `current_type` (depth-10, cycle-guarded)
/// retrying the member on each ancestor. Returns the next chain type when an
/// inherited field/method matches, advancing `env` for any new generic args.
fn walk_inheritance_for_member(
    current_type: &str,
    member_name: &str,
    config: &ChainConfig,
    lookup: &dyn SymbolLookup,
    mut env: Option<&mut TypeEnvironment>,
) -> Option<String> {
    let mut ancestor = current_type.to_string();
    for _ in 0..10 {
        let parent = lookup.parent_class_qname(&ancestor)?.to_string();
        let parent_member = format!("{parent}.{member_name}");
        if let Some(next) = lookup.field_type_str(&parent_member) {
            let new_args = lookup.field_type_arg_strs(&parent_member).unwrap_or_default();
            let resolved = resolve_and_enter_generics_args(
                &next, &new_args, config, lookup, env.as_deref_mut(),
            );
            return Some(resolved);
        }
        if let Some(next) = lookup.return_type_str(&parent_member) {
            let resolved = resolve_and_enter_generics(
                &next, &parent_member, config, lookup, env.as_deref_mut(), false,
            );
            return Some(resolved);
        }
        for sym in lookup.members_of(&parent) {
            if sym.name != member_name {
                continue;
            }
            if let Some(ft) = lookup.field_type_str(&sym.qualified_name) {
                return Some(resolve_and_enter_generics(
                    &ft, &sym.qualified_name, config, lookup, env.as_deref_mut(), true,
                ));
            }
            if let Some(rt) = lookup.return_type_str(&sym.qualified_name) {
                return Some(resolve_and_enter_generics(
                    &rt, &sym.qualified_name, config, lookup, env.as_deref_mut(), false,
                ));
            }
        }
        if parent == ancestor {
            break;
        }
        ancestor = parent;
    }
    None
}

/// Resolve a type through the TypeEnvironment (if active) and enter a new
/// generic context bound to explicit `new_args` (used by the inheritance walk,
/// where the args come from a `field_type_args` lookup rather than the member
/// qname). Mirrors `resolve_and_enter_generics`'s field branch.
fn resolve_and_enter_generics_args(
    raw_type: &str,
    new_args: &[String],
    config: &ChainConfig,
    lookup: &dyn SymbolLookup,
    env: Option<&mut TypeEnvironment>,
) -> String {
    let normalized = (config.normalize_type)(raw_type);
    if let Some(env) = env {
        let resolved = env.resolve(&normalized);
        env.push_scope();
        if !new_args.is_empty() {
            env.enter_generic_context(&resolved, new_args, |name| {
                lookup.generic_params(name).map(|p| p.to_vec())
            });
        }
        resolved
    } else {
        normalized
    }
}

/// Resolve a type through the TypeEnvironment (if active) and optionally
/// enter a new generic context for the resolved type.
fn resolve_and_enter_generics(
    raw_type: &str,
    member_qname: &str,
    config: &ChainConfig,
    lookup: &dyn SymbolLookup,
    env: Option<&mut TypeEnvironment>,
    is_field: bool,
) -> String {
    let normalized = (config.normalize_type)(raw_type);
    if let Some(env) = env {
        let resolved = env.resolve(&normalized);
        env.push_scope();
        if is_field {
            let new_args = lookup
                .field_type_args(member_qname)
                .unwrap_or(&[])
                .to_vec();
            if !new_args.is_empty() {
                env.enter_generic_context(&resolved, &new_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }
        }
        resolved
    } else {
        normalized
    }
}

/// Try namespace-qualified lookup for intermediate segments (Java, C#, PHP).
fn resolve_via_namespace(
    config: &ChainConfig,
    file_ctx: &FileContext,
    member_qname: &str,
    lookup: &dyn SymbolLookup,
    mut env: Option<&mut TypeEnvironment>,
) -> Option<String> {
    for import in &file_ctx.imports {
        let use_this = match config.namespace_lookup {
            NamespaceLookup::WildcardOnly | NamespaceLookup::WildcardWithGenerics => {
                import.is_wildcard
            }
            NamespaceLookup::AllImports => true,
            NamespaceLookup::None => false,
        };
        if !use_this {
            continue;
        }
        let Some(module) = &import.module_path else {
            continue;
        };
        let qualified = format!("{module}.{member_qname}");

        if let Some(next_type) = lookup.field_type_name(&qualified) {
            let resolved = if let Some(env) = env.as_deref_mut() {
                let r = env.resolve(&(config.normalize_type)(next_type));
                env.push_scope();
                r
            } else {
                (config.normalize_type)(next_type)
            };
            return Some(resolved);
        }
        if let Some(next_type) = lookup.return_type_name(&qualified) {
            let resolved = if let Some(env) = env.as_deref_mut() {
                let r = env.resolve(&(config.normalize_type)(next_type));
                env.push_scope();
                r
            } else {
                (config.normalize_type)(next_type)
            };
            return Some(resolved);
        }
    }
    None
}

/// Try namespace-qualified lookup for the final segment (Java, C#, PHP).
fn resolve_final_via_namespace(
    config: &ChainConfig,
    file_ctx: &FileContext,
    current_type: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let candidate = format!("{current_type}.{target_name}");
    for import in &file_ctx.imports {
        let use_this = match config.namespace_lookup {
            NamespaceLookup::WildcardOnly | NamespaceLookup::WildcardWithGenerics => {
                import.is_wildcard
            }
            NamespaceLookup::AllImports => true,
            NamespaceLookup::None => false,
        };
        if !use_this {
            continue;
        }
        let Some(module) = &import.module_path else {
            continue;
        };
        let ns_candidate = format!("{module}.{candidate}");
        if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
            if (config.kind_compatible)(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: chain_strategy(config.strategy_prefix),
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    None
}

/// Build the strategy name for diagnostics.
fn chain_strategy(prefix: &str) -> &'static str {
    match prefix {
        "ts" => "ts_chain_resolution",
        "python" => "python_chain_resolution",
        "rust" => "rust_chain_resolution",
        "go" => "go_chain_resolution",
        "csharp" => "csharp_chain_resolution",
        "java" => "java_chain_resolution",
        "php" => "php_chain_resolution",
        "ruby" => "ruby_chain_resolution",
        "kotlin" => "kotlin_chain_resolution",
        "scala" => "scala_chain_resolution",
        "dart" => "dart_chain_resolution",
        "swift" => "swift_chain_resolution",
        "c" => "c_chain_resolution",
        "starlark" => "starlark_chain_resolution",
        _ => "chain_resolution",
    }
}

/// Strategy name for an inherited final-segment hit — the member lives on an
/// ancestor reached through the `extends` chain rather than the receiver type.
fn chain_strategy_inheritance(prefix: &str) -> &'static str {
    match prefix {
        "ts" => "ts_chain_inheritance",
        "csharp" => "csharp_chain_inheritance",
        "java" => "java_chain_inheritance",
        "go" => "go_chain_inheritance",
        "php" => "php_chain_inheritance",
        "c" => "c_chain_inheritance",
        "kotlin" => "kotlin_chain_inheritance",
        "scala" => "scala_chain_inheritance",
        "dart" => "dart_chain_inheritance",
        "swift" => "swift_chain_inheritance",
        _ => "chain_inheritance",
    }
}

/// Strategy name for an extension-method final-segment hit — the member is a
/// static method with a `this`-qualified receiver matching the chain's type.
fn chain_strategy_extension(prefix: &str) -> &'static str {
    match prefix {
        "csharp" => "csharp_extension_method",
        _ => "chain_extension_method",
    }
}

/// Strategy name for the unique prefix-match variant — exactly one symbol
/// within the resolved type owns the trailing segment, so the resolution
/// is deterministic and emitted at confidence 1.0.
fn chain_strategy_unique(prefix: &str) -> &'static str {
    match prefix {
        "ts" => "ts_chain_resolution_unique",
        "python" => "python_chain_resolution_unique",
        "rust" => "rust_chain_resolution_unique",
        "go" => "go_chain_resolution_unique",
        "csharp" => "csharp_chain_resolution_unique",
        "java" => "java_chain_resolution_unique",
        "php" => "php_chain_resolution_unique",
        "ruby" => "ruby_chain_resolution_unique",
        "kotlin" => "kotlin_chain_resolution_unique",
        "scala" => "scala_chain_resolution_unique",
        "dart" => "dart_chain_resolution_unique",
        "swift" => "swift_chain_resolution_unique",
        "c" => "c_chain_resolution_unique",
        "starlark" => "starlark_chain_resolution_unique",
        _ => "chain_resolution_unique",
    }
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
