// =============================================================================
// indexer/resolve/chain_walker.rs — Unified chain-aware resolution
//
// Replaces 8 per-language chain.rs files with a single parameterized
// implementation. Language differences are captured in `ChainConfig`,
// not duplicated code.
//
// Three-phase algorithm:
//   Phase 1: Determine root type (SelfRef → enclosing class, Identifier → field type)
//   Phase 2: Walk intermediate segments following field_type_name / return_type_name
//   Phase 3: Resolve final segment on the resolved type
// =============================================================================

use crate::indexer::resolve::engine::{
    ChainMiss, FileContext, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::resolve::type_env::TypeEnvironment;
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
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;

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
                    if let Some(type_name) = lookup.field_type_name(&field_qname) {
                        if config.use_generics {
                            initial_generic_args = lookup
                                .field_type_args(&field_qname)
                                .unwrap_or(&[])
                                .to_vec();
                        }
                        found = Some((config.normalize_type)(type_name));
                        break;
                    }
                }
                found.or_else(|| {
                    segments[0]
                        .declared_type
                        .as_ref()
                        .map(|t| (config.normalize_type)(t))
                })
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

    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        // Try field type (property access).
        if let Some(next_type) = lookup.field_type_name(&member_qname) {
            let resolved = resolve_and_enter_generics(
                next_type,
                &member_qname,
                config,
                lookup,
                env.as_mut(),
                true,
            );
            current_type = resolved;
            continue;
        }

        // Try return type (method call result in a fluent chain).
        if let Some(raw_return) = lookup.return_type_name(&member_qname) {
            let resolved = resolve_and_enter_generics(
                raw_return,
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
            if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                let resolved = resolve_and_enter_generics(
                    ft,
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
            if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                let resolved = resolve_and_enter_generics(
                    rt,
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

        // Lost the chain — can't determine the next type. Record the miss
        // for R3 lazy reload: a second pass will call resolve_symbol on
        // `current_type`'s owning ecosystem dep to pull its definition file.
        lookup.record_chain_miss(ChainMiss {
            current_type: current_type.clone(),
            target_name: seg.name.clone(),
        });
        return None;
    }

    // ------------------------------------------------------------------
    // Phase 3: Resolve the final segment on the resolved type.
    // ------------------------------------------------------------------
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    // Direct qualified name match.
    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if (config.kind_compatible)(edge_kind, &sym.kind) {
            debug!(
                strategy = %format!("{strategy}_chain_resolution"),
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: chain_strategy(strategy),
            });
        }
    }

    // Namespace-qualified final resolution (Java, C#, PHP).
    if config.namespace_lookup != NamespaceLookup::None {
        if let Some(file_ctx) = file_ctx {
            if let Some(res) = resolve_final_via_namespace(
                config, file_ctx, &current_type, &last.name, edge_kind, lookup,
            ) {
                return Some(res);
            }
        }
    }

    // by_name scoped to the resolved type.
    //
    // Constrain the prefix match: `current_type = "Foo"` must be followed by
    // a `.` so it doesn't spuriously collide with `FooBar.bar`. Then collect
    // all matches — a single match is deterministic enough to emit at
    // confidence 1.0 via a dedicated "*_chain_resolution_unique" strategy,
    // while multiple matches keep the 0.95 hedge.
    let type_prefix = format!("{current_type}.");
    let matches: Vec<&SymbolInfo> = lookup
        .by_name(&last.name)
        .iter()
        .filter(|sym| {
            (sym.qualified_name == current_type
                || sym.qualified_name.starts_with(&type_prefix))
                && (config.kind_compatible)(edge_kind, &sym.kind)
        })
        .collect();
    match matches.len() {
        0 => {}
        1 => {
            return Some(Resolution {
                target_symbol_id: matches[0].id,
                confidence: 1.0,
                strategy: chain_strategy_unique(strategy),
            });
        }
        _ => {
            return Some(Resolution {
                target_symbol_id: matches[0].id,
                confidence: 0.95,
                strategy: chain_strategy(strategy),
            });
        }
    }

    // Final-segment miss: walked to current_type but no `.last.name` found
    // anywhere under it. Same R3 reload signal as the intermediate-segment
    // bail-out above.
    lookup.record_chain_miss(ChainMiss {
        current_type: current_type.clone(),
        target_name: last.name.clone(),
    });
    None
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
        _ => "chain_resolution",
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
        _ => "chain_resolution_unique",
    }
}
