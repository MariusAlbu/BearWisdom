// =============================================================================
// languages/rust_lang/hooks.rs — RustHooks impl plus the concrete RustResolver
// (Self/Self::X type resolution, generic-param + turbofish + 2-letter generic
// filtering, chain via RustChecker, module-field qualified call resolution,
// scope-chain, same-file, same-module via crate path, use-statement
// resolution with :: to . normalization, fully-qualified path with
// crate/super rewriting, field-type chain) plus 15 flow detectors
// (Axum routes + WS Consumer, Actix resources, route attribute macros,
// async-graphql, Tauri command, reqwest client, Diesel ORM, sqlx macro,
// Lettre mailer, rdkafka MQ, redis config lookup, apalis bgjob, tonic gRPC,
// UDS) plus external classifier (Cargo manifest match + has_in_namespace
// fallback) plus build_file_context with use-statement parsing.
// =============================================================================
pub(crate) use super::flow_detectors::{
    detect_rust_actix_resource_emission, detect_rust_apalis_bgjob, detect_rust_async_graphql_attribute,
    detect_rust_axum_route_emission, detect_rust_axum_ws_consumer, detect_rust_diesel_emission,
    detect_rust_lettre_mailer, detect_rust_rdkafka_mq, detect_rust_redis_config_lookup,
    detect_rust_reqwest_emission, detect_rust_route_attribute_emission, detect_rust_sqlx_macro_emission,
    detect_rust_tauri_command_attribute, detect_rust_tonic_emission, detect_rust_uds_emission,
};
use super::{keywords, predicates};
use super::predicates::normalize_path;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    intern_yield_type, ChainMiss, FileContext, ImportEntry, RefContext, Resolution, SymbolInfo,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::type_checker::chain::simple_yield_type;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

/// Rust language resolver.
pub struct RustResolver;

impl RustResolver {

    
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
pub(crate) fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Skip import refs — they declare scope, not symbol references.

        // `Self` → resolve to the enclosing struct/enum/trait.
        if target == "Self" {
            let enclosing = find_enclosing_type(&ref_ctx.scope_chain, lookup)?;
            let sym = lookup.by_qualified_name(&enclosing)?;
            if predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "rust_self_type",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
            // For Calls to Self (e.g. Self::new()), look for a method
            // on the enclosing type.
            if edge_kind == EdgeKind::Calls {
                // Try to find a constructor or associated function.
                for child in lookup.in_namespace(&enclosing) {
                    if child.name == "new"
                        && matches!(child.kind.as_str(), "method" | "function" | "constructor")
                    {
                        return Some(Resolution {
                            target_symbol_id: child.id,
                            confidence: 0.95,
                            strategy: "rust_self_constructor",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            // Even if we can't find the exact method, resolve to the type itself.
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "rust_self_type_fallback",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }

        // `Self::X` — TypeRef / Calls where the extractor captured the leaf
        // name in `target` and the prefix in `module`. Treat as a member
        // lookup against the enclosing struct/enum/trait. This rescues enum
        // variant references like `Self::Named` / `Self::Tuple` inside `impl`
        // blocks, which the chain walker can't see when the type-ref pass
        // emits a chain-less ref.
        if ref_ctx.extracted_ref.module.as_deref() == Some("Self") {
            if let Some(enclosing) =
                find_enclosing_type(&ref_ctx.scope_chain, lookup)
            {
                let candidate = format!("{enclosing}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "rust_self_scoped",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }

        // Generic type parameters: single uppercase letters in TypeRef position
        // (e.g., `L`, `M`, `F`, `W`) are generic params, never indexable symbols.
        if edge_kind == EdgeKind::TypeRef {
            let bare = target.trim_start_matches("::");
            if bare.len() == 1 && bare.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
                return None;
            }
        }

        // Turbofish / generic type argument targets: `<Vec<_>>`, `<MyMessage>`,
        // `<i64, usize>` etc. are extractor noise — type args mis-emitted as Calls.
        // The leading `<` is the reliable marker; bail early.
        if target.starts_with('<') {
            return None;
        }

        // Two-uppercase-letter numeric suffix generics (P1, T2, etc.) are almost
        // always generic type parameters, not real symbols.
        if target.len() == 2 {
            let mut chars = target.chars();
            let (a, b) = (chars.next().unwrap(), chars.next().unwrap());
            if a.is_ascii_uppercase() && b.is_ascii_digit() {
                return None;
            }
        }

        // Chain-aware resolution.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = walk_rust_lang_chain(chain_val, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // Module-field resolution for qualified call refs (e.g. `DbPool::new()`
        // where the extractor post-pass set module="crate::db").
        // Only fires for Calls and TypeRef — not Imports (handled separately above).
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef) {
            if let Some(module) = &ref_ctx.extracted_ref.module {
                if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
                    if chain_val.segments.len() >= 2 {
                        let type_name = &chain_val.segments[0].name;
                        // "{type_name}.{target}" — standard Rust method storage form.
                        let candidate = format!("{type_name}.{target}");
                        if let Some(sym) = lookup.by_qualified_name(&candidate) {
                            if predicates::kind_compatible(edge_kind, &sym.kind) {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 1.0,
                                    strategy: "rust_ref_module",
                                    resolved_yield_type: None,
                                    flow_emit: None,
                                });
                            }
                        }
                        // "{last_module_segment}.{type_name}.{target}" — module-qualified form.
                        let last_seg = module.rsplit("::").next().unwrap_or(module.as_str());
                        let candidate2 = format!("{last_seg}.{type_name}.{target}");
                        if let Some(sym) = lookup.by_qualified_name(&candidate2) {
                            if predicates::kind_compatible(edge_kind, &sym.kind) {
                                return Some(Resolution {
                                    target_symbol_id: sym.id,
                                    confidence: 1.0,
                                    strategy: "rust_ref_module",
                                    resolved_yield_type: None,
                                    flow_emit: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Normalize `::` separators to `.` for index lookup.
        let normalized = predicates::normalize_path(target);
        let effective_target = &normalized;

        // Step 1: Scope chain walk (innermost → outermost).
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible_with_signature(edge_kind, sym)
                {
                    let strategy = if sym.kind == "variable" {
                        "rust_scope_chain_callable_var"
                    } else {
                        "rust_scope_chain"
                    };
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy,
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 2: Same-module resolution.
        // Symbols in the same module are visible without `use`.
        if let Some(module) = &file_ctx.file_namespace {
            let candidate = format!("{module}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_same_module",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // By simple name, preferring same module.
            let candidates = lookup.by_name(effective_target);
            for sym in candidates {
                if predicates::sym_module(sym) == module.as_str()
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_same_module_by_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 3: Import-based resolution.
        // `use foo::bar::Baz` → look for `Baz` in the symbol index,
        // preferring symbols whose qualified_name starts with the import's module path.
        for import in &file_ctx.imports {
            if import.is_wildcard {
                // Wildcard: find anything in the imported module matching the name.
                if let Some(ref mod_path) = import.module_path {
                    let dot_mod = predicates::normalize_path(mod_path);
                    let candidate = format!("{dot_mod}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if self.is_visible(file_ctx, ref_ctx, sym)
                            && predicates::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "rust_wildcard_import",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
                    }
                }
                continue;
            }

            // Named import: the imported_name must match.
            if import.imported_name != *effective_target {
                continue;
            }

            let Some(ref mod_path) = import.module_path else {
                continue;
            };

            let dot_mod = predicates::normalize_path(mod_path);

            // Try {module}.{name} — most common Rust qualified name form.
            let candidate = format!("{dot_mod}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // Also try: just the name, scoped to the module prefix.
            // Two variants: with and without leading "crate." since the symbol
            // index never includes the "crate." prefix in qualified names.
            // Require a trailing "." so "models" doesn't match "modelsfoo.User".
            let dot_mod_stripped = dot_mod
                .strip_prefix("crate.")
                .unwrap_or(dot_mod.as_str());
            let dot_mod_prefix = format!("{}.", dot_mod);
            let dot_mod_stripped_prefix = format!("{}.", dot_mod_stripped);
            for sym in lookup.by_name(effective_target) {
                let qn = sym.qualified_name.as_str();
                let prefix_match = qn.starts_with(dot_mod_prefix.as_str())
                    || qn.starts_with(dot_mod_stripped_prefix.as_str());
                if prefix_match
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "rust_import_prefix",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // Re-export fallback: `pub use submod::Type` means the symbol lives
            // in a submodule of the imported module. Map the module path to a
            // directory segment and find candidates whose file path sits under
            // that directory subtree.
            //
            // "crate::models" → "models/" (forward-slash, any position in path)
            // "crate::api::handlers" → "api/handlers/"
            let dir_suffix: String = {
                let segs: Vec<&str> = dot_mod_stripped
                    .split('.')
                    .filter(|s| !s.is_empty())
                    .collect();
                if segs.is_empty() {
                    String::new()
                } else {
                    format!("{}/", segs.join("/"))
                }
            };
            if !dir_suffix.is_empty() {
                let candidates: Vec<&crate::indexer::resolve::engine::SymbolInfo> =
                    lookup.by_name(effective_target)
                        .iter()
                        .filter(|sym| {
                            // file_path contains the module's directory anywhere in the path,
                            // using forward slashes (the index normalizes to `/`).
                            let fp = sym.file_path.replace('\\', "/");
                            fp.contains(dir_suffix.as_str())
                                && self.is_visible(file_ctx, ref_ctx, sym)
                                && predicates::kind_compatible(edge_kind, &sym.kind)
                        })
                        .collect();
                if candidates.len() == 1 {
                    return Some(Resolution {
                        target_symbol_id: candidates[0].id,
                        confidence: 0.90,
                        strategy: "rust_reexport_dir",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
                // Multiple candidates: prefer the one whose qualified_name is shortest
                // (closest to the module root — less nesting = less ambiguity).
                if !candidates.is_empty() {
                    let best = candidates
                        .iter()
                        .min_by_key(|s| s.qualified_name.len())
                        .unwrap();
                    return Some(Resolution {
                        target_symbol_id: best.id,
                        confidence: 0.85,
                        strategy: "rust_reexport_dir_ambiguous",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 4: Crate-qualified resolution.
        // `crate::foo::Bar` or `foo::Bar` with `::` separators.
        if target.contains("::") || effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "rust_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // Strip leading "crate." and try again.
            let stripped = effective_target
                .strip_prefix("crate.")
                .unwrap_or(effective_target);
            if stripped != effective_target {
                if let Some(sym) = lookup.by_qualified_name(stripped) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "rust_qualified_name_stripped",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }

        // Step 5: Same-file by-name for Calls.
        // When multiple candidates share the name, prefer the one in the
        // same file as the caller — common for per-file helper functions
        // (e.g. `test_match` defined locally in each language module).
        if edge_kind == EdgeKind::Calls {
            let candidates: Vec<&SymbolInfo> = lookup
                .by_name(effective_target)
                .into_iter()
                .filter(|s| predicates::kind_compatible(edge_kind, &s.kind))
                .collect();
            let same_file: Vec<&&SymbolInfo> = candidates
                .iter()
                .filter(|s| s.file_path.as_ref() == file_ctx.file_path.as_str())
                .collect();
            if same_file.len() == 1 {
                return Some(Resolution {
                    target_symbol_id: same_file[0].id,
                    confidence: 0.90,
                    strategy: "rust_same_file_name_fallback",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Step 6: Rust prelude. Names in `std::prelude::v1` are in scope
        // without an explicit `use`. The earlier steps fail when an
        // identically-named internal symbol (e.g. an `<Enum>.Vec`
        // variant) makes the by-name lookup ambiguous, or when the
        // prelude entity is one of many external crates re-exporting
        // the same identifier. Prefer the `rust-stdlib` candidate.
        if matches!(
            edge_kind,
            EdgeKind::Calls
                | EdgeKind::TypeRef
                | EdgeKind::Instantiates
                | EdgeKind::Implements
                | EdgeKind::Inherits
        )
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains("::")
            && !target.contains('.')
            && keywords::PRELUDE_NAMES.contains(&target.as_str())
        {
            // Variants are stored under their parent enum's qname:
            // `Option.Some`, `Result.Ok`, `Result.Err`. Probe those
            // first when the target is a prelude variant.
            let variant_qname = match target.as_str() {
                "Some" | "None" => Some(format!("Option.{target}")),
                "Ok" | "Err" => Some(format!("Result.{target}")),
                _ => None,
            };
            if let Some(qn) = variant_qname.as_deref() {
                for sym in lookup.all_by_qualified_name(qn) {
                    // Variants of `Option` / `Result` are constructed via call
                    // syntax (`Some(x)`, `Ok(y)`) and pattern-matched (`Some(_)` in
                    // a match arm). Both shapes land here — accept enum_member
                    // regardless of edge_kind for the four prelude variants.
                    let compat = sym.kind == "enum_member"
                        || predicates::kind_compatible(edge_kind, &sym.kind);
                    if compat && keywords::is_rust_stdlib_path(&sym.file_path) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "rust_prelude",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }

            // Bare-name shape: trait/type/macro lives at the leaf qname
            // (`Vec`, `Box`, `Default`, `format`, `println`, ...). The
            // stdlib walker is authoritative; ignore same-named symbols
            // from third-party crates and from internal collisions.
            for sym in lookup.by_name(target) {
                if keywords::is_rust_stdlib_path(&sym.file_path)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "rust_prelude",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Rust bare-name fallback. Continues the cross-language
        // template (PRs 31, 35-40, Lua, Go). Rust's `use` brings
        // names into scope and trait methods are callable by bare
        // name — both can leak past the deterministic path when
        // type inference can't fully bind. Also fires for
        // `Implements` edges produced by `impl Trait for Type` —
        // those have no chain context and no module-field path
        // when the extractor stored the qualifier elsewhere
        // (`impl std::ops::Deref` produces target_name="Deref"
        // because the resolver post-pass split the path). Gated
        // by `.rs` file extension and `kind_compatible`.
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if matches!(
            edge_kind,
            EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates | EdgeKind::Implements
        )
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains("::")
            && !target.contains('.')
        {
            // Name-only chain miss: every project resolution path failed,
            // but the target might live in an external Rust file the
            // demand-driven cargo walker hasn't parsed yet (only
            // `scan_rust_header` ran on it). Record a chain miss with
            // empty `current_type` so `expand.rs::locate_via_symbol_index`
            // falls through to its phase-B bare-name probe against the
            // SymbolLocationIndex. If a hit exists, the file is pulled,
            // parsed, and a second resolve pass picks up the symbol via
            // the same bare-name fallback above.
            //
            // Real motivator: `x.into_response()` against axum's
            // `IntoResponse::into_response`. The axum file is walked
            // (`mod`-tree expansion catches it) but never parsed, because
            // the chain walker can't infer the `impl IntoResponse`
            // receiver type and so emits no chain miss. With this
            // recording, the bare name `into_response` is enough to
            // locate the file.
            //
            // Bounded by the location index: only names that exist
            // somewhere external trigger a file pull. Real typos and
            // bare-noise refs cost a hash lookup and bail.
            let trivial = target.len() < 2
                || target.chars().next().map_or(true, |c| c == '_')
                || !target.chars().any(|c| c.is_alphabetic());
            if !trivial {
                lookup.record_chain_miss(
                    crate::indexer::resolve::engine::ChainMiss {
                        current_type: String::new(),
                        target_name: target.clone(),
                    },
                );
            }
        }

        None
    }

    pub(crate) fn is_visible(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _target: &SymbolInfo,
    ) -> bool {
        // Navigation tool: visibility never gates resolution, so go-to-definition
        // reaches private members. Deliberate divergence from compiler behavior.
        true
    }

}

/// Rust chain walker.
pub(crate) fn walk_rust_lang_chain(
    chain_ref: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain_ref.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => find_enclosing_type(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            if let Some(local_type) = lookup.local_type(name) {
                Some(normalize_path(&local_type))
            } else {
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "struct" | "enum" | "trait" | "type_alias" | "class"
                    )
                });
                if is_type {
                    Some(normalize_path(name))
                } else {
                    let mut found = None;
                    for scope in &ref_ctx.scope_chain {
                        let field_qname = format!("{scope}.{name}");
                        if let Some(type_name) = lookup.field_type_str(&field_qname) {
                            found = Some(normalize_path(&type_name));
                            break;
                        }
                    }
                    found.or_else(|| {
                        segments[0].declared_type.as_ref().map(|t| normalize_path(t))
                    })
                }
            }
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // Phase 2: intermediate segments.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_str(&member_qname) {
            current_type = normalize_path(&next_type);
            continue;
        }
        if let Some(next_type) = lookup.return_type_str(&member_qname) {
            current_type = normalize_path(&next_type);
            continue;
        }

        let mut found = false;
        for sym in lookup.members_of(&current_type) {
            if sym.name != seg.name {
                continue;
            }
            if let Some(ft) = lookup.field_type_str(&sym.qualified_name) {
                current_type = normalize_path(&ft);
                found = true;
                break;
            }
            if let Some(rt) = lookup.return_type_str(&sym.qualified_name) {
                current_type = normalize_path(&rt);
                found = true;
                break;
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

    // Phase 3: final segment.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if predicates::kind_compatible(edge_kind, &sym.kind) {
            tracing::debug!(
                strategy = "rust_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "rust_chain_resolution",
                resolved_yield_type: intern_yield_type(
                    simple_yield_type(sym, lookup).map(|t| normalize_path(&t)),
                    lookup,
                ),
                flow_emit: None,
            });
        }
    }

    for sym in lookup.members_of(&current_type) {
        if sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "rust_chain_resolution",
                resolved_yield_type: intern_yield_type(
                    simple_yield_type(sym, lookup).map(|t| normalize_path(&t)),
                    lookup,
                ),
                flow_emit: None,
            });
        }
    }

    // Inheritance walk: `current_type` may inherit `last.name` from a parent
    // (trait default method, struct extending another via inheritance map).
    // Generic helper that climbs `parent_class_qname` and retries the lookup.
    if let Some(sym) = crate::indexer::resolve::engine::find_member_via_inheritance(
        &current_type,
        &last.name,
        edge_kind,
        lookup,
        predicates::kind_compatible,
    ) {
        return Some(Resolution {
            target_symbol_id: sym.id,
            confidence: 0.90,
            strategy: "rust_chain_inheritance",
            resolved_yield_type: intern_yield_type(
                simple_yield_type(sym, lookup).map(|t| normalize_path(&t)),
                lookup,
            ),
            flow_emit: None,
});
    }

    lookup.record_chain_miss(ChainMiss {
        current_type: current_type.clone(),
        target_name: last.name.clone(),
    });
    None
}

/// Find the enclosing struct/impl/trait name from the scope chain.
fn find_enclosing_type(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "struct" | "enum" | "trait" | "class") {
                return Some(scope.clone());
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}

pub(super) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

        // Import refs: `use serde::Deserialize` → classify by the first crate segment.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            // First segment of a `::` path identifies the crate.
            let first = import_path.split("::").next().unwrap_or(import_path);
            if matches!(first, "crate" | "self" | "super") {
                return None; // internal
            }
            if keywords::STDLIB_CRATES.contains(&first) {
                return Some("std".to_string());
            }
            let name = first;
            // Manifest-driven: check Cargo.toml dependencies first.
            // Crate names may use hyphens in Cargo.toml but underscores in source.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Cargo) {
                    if manifest.dependencies.contains(name)
                        || manifest.dependencies.contains(&name.replace('_', "-"))
                    {
                        return Some(first.to_string());
                    }
                }
            }
            let is_ext = match project_ctx {
                Some(ctx) => is_manifest_rust_crate(ctx, name),
                None => true, // conservative: treat as external
            };
            if is_ext {
                return Some(first.to_string());
            }
            return None;
        }

        // Bare `::`-paths without a matching `use` import (e.g. inline
        // `anyhow::anyhow!()` or `tracing::info!()`): consult Cargo.toml.
        // The manifest is authoritative for crate attribution.
        if target.contains("::") {
            let first = target.split("::").next().unwrap_or("");
            if !first.is_empty() && !matches!(first, "crate" | "self" | "super") {
                if keywords::STDLIB_CRATES.contains(&first) {
                    return Some("std".to_string());
                }
                if let Some(ctx) = project_ctx {
                    if is_manifest_rust_crate(ctx, first) {
                        return Some(first.to_string());
                    }
                }
            }
        }

        // For non-import refs, check if the target came from an external import.
        // Walk the file's import list, matching either:
        //   - the simple target name (`use serde::Deserialize;` → `Deserialize`)
        //   - the first segment of the ref's module path (`fmt::Formatter`
        //     with `use std::fmt;` → `fmt`)
        let normalized = predicates::normalize_path(target);
        let simple = normalized.split('.').next_back().unwrap_or(&normalized);
        let module_root = ref_ctx
            .extracted_ref
            .module
            .as_deref()
            .and_then(|m| m.split("::").next())
            .filter(|s| !s.is_empty());

        for import in &file_ctx.imports {
            if import.imported_name != simple
                && Some(import.imported_name.as_str()) != module_root
            {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };
            let first = mod_path.split("::").next().unwrap_or(mod_path);
            if matches!(first, "crate" | "self" | "super") {
                continue;
            }
            if keywords::STDLIB_CRATES.contains(&first) {
                return Some("std".to_string());
            }
            let name = first;
            // Manifest-driven check.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Cargo) {
                    if manifest.dependencies.contains(name)
                        || manifest.dependencies.contains(&name.replace('_', "-"))
                    {
                        return Some(first.to_string());
                    }
                }
            }
            let is_ext = match project_ctx {
                Some(ctx) => is_manifest_rust_crate(ctx, name),
                None => true,
            };
            if is_ext {
                return Some(first.to_string());
            }
        }

        // Structural fallback: any walk-imports candidate from a crate
        // the index has no symbols for is external. Catches re-exports
        // (`pub use foo::Bar` where Bar lives in an external crate that
        // isn't in the symbol set) and dev-deps that the manifest pass
        // missed. Only fires when called via the `_with_lookup` variant.
        if let Some(lookup) = lookup {
            for import in &file_ctx.imports {
                if import.imported_name != simple
                    && Some(import.imported_name.as_str()) != module_root
                {
                    continue;
                }
                let Some(ref mod_path) = import.module_path else {
                    continue;
                };
                let first = mod_path.split("::").next().unwrap_or(mod_path);
                if matches!(first, "crate" | "self" | "super") {
                    continue;
                }
                if !lookup.has_in_namespace(first) {
                    return Some(first.to_string());
                }
            }
        }

        // Wildcard-import fallback: `use proptest::prelude::*;` brings
        // arbitrary names into scope. As the last resort, attribute
        // unresolved targets to the first external wildcard-imported
        // crate (manifest-known or has_in_namespace external). Lossy
        // attribution by design — without trait/symbol resolution
        // through external `.rs`, we can't tell which wildcard sourced
        // the name. Still better than `unresolved_refs`.
        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };
            let first = mod_path.split("::").next().unwrap_or(mod_path);
            if matches!(first, "crate" | "self" | "super") {
                continue;
            }
            if keywords::STDLIB_CRATES.contains(&first) {
                return Some("std".to_string());
            }
            if let Some(ctx) = project_ctx {
                if is_manifest_rust_crate(ctx, first) {
                    return Some(first.to_string());
                }
            }
            if let Some(lookup) = lookup {
                if !lookup.has_in_namespace(first) {
                    return Some(first.to_string());
                }
            }
        }

        // Builder chain propagation: if the ref has a chain and the root segment
        // was imported from an external crate, classify the whole chain external.
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if chain_ref.segments.len() >= 2 {
                let root = &chain_ref.segments[0].name;
                for import in &file_ctx.imports {
                    if import.imported_name != root.as_str() {
                        continue;
                    }
                    if let Some(ref mod_path) = import.module_path {
                        let first = mod_path.split("::").next().unwrap_or(mod_path);
                        let is_ext = if matches!(first, "crate" | "self" | "super") {
                            false
                        } else if keywords::STDLIB_CRATES.contains(&first) {
                            true
                        } else {
                            match project_ctx {
                                Some(ctx) => is_manifest_rust_crate(ctx, first),
                                None => true,
                            }
                        };
                        if is_ext {
                            return Some(format!("{}.*", first));
                        }
                    }
                }
            }
        }

    // Final externals-index fallback: if every project import / wildcard /
    // chain check failed, try a by-name lookup. When `target_name` matches
    // one or more symbols defined in external files (`ext:` path prefix),
    // classify the ref as external. Covers common stdlib method calls
    // (`trim_matches`, `to_string_lossy`, `to_path_buf`, `with_context`,
    // `is_ascii_alphanumeric`) that aren't directly imported but are
    // walked in via rust-stdlib / cargo deps.
    //
    // Conservative: only fires when EVERY matching symbol is external.
    // If any match is project-internal, we let resolution fall through
    // and either resolve to that internal symbol or land as unresolved
    // — `trim_matches`-style names occasionally collide with user-defined
    // helpers, and we don't want to misattribute a real bug to "external".
    if let Some(lookup) = lookup {
        let matches = lookup.by_name(target);
        if !matches.is_empty() {
            let all_external = matches
                .iter()
                .all(|s| s.file_path.starts_with("ext:"));
            if all_external {
                // Pick a representative crate from the first match's file path.
                // `ext:rust:.../core/src/str/mod.rs` → `core`. Fall back to
                // "std" if we can't parse it cleanly — the classification
                // matters for grouping, not for chain walking.
                let crate_name = matches
                    .first()
                    .and_then(|s| classify_external_path_as_crate(&s.file_path))
                    .unwrap_or_else(|| "std".to_string());
                return Some(crate_name);
            }
        }
    }

    None
}

/// Extract a representative crate name from an `ext:` virtual path. Used
/// purely for classification; `"std"` is the fallback when the path shape
/// doesn't match a known ecosystem layout.
fn classify_external_path_as_crate(path: &str) -> Option<String> {
    // Common shapes:
    //   ext:rust:<sysroot>/lib/rustlib/src/rust/library/<crate>/...   → <crate>
    //   ext:rust:<cargo-cache>/registry/src/<reg>/<crate>-<ver>/...    → <crate>
    //   ext:rust:<other>                                              → "std" fallback
    let stripped = path.strip_prefix("ext:rust:").unwrap_or(path);
    // Find a `/library/` segment (rust-stdlib).
    if let Some(rest) = stripped
        .split_once("/library/")
        .map(|(_, after)| after)
    {
        if let Some(crate_seg) = rest.split('/').next() {
            if !crate_seg.is_empty() {
                return Some(crate_seg.to_string());
            }
        }
    }
    // Find a `/registry/src/<reg>/<crate>-<ver>/` segment (cargo cache).
    if let Some((_, after)) = stripped.split_once("/registry/src/") {
        // Skip the registry name.
        if let Some((_, after_reg)) = after.split_once('/') {
            if let Some(crate_dir) = after_reg.split('/').next() {
                // `<crate>-<ver>` → crate is everything before the last `-`.
                if let Some(idx) = crate_dir.rfind('-') {
                    let crate_name = &crate_dir[..idx];
                    if !crate_name.is_empty() {
                        return Some(crate_name.to_string());
                    }
                }
                if !crate_dir.is_empty() {
                    return Some(crate_dir.to_string());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a Rust crate name is an external dependency using the Cargo manifest.
fn is_manifest_rust_crate(ctx: &ProjectContext, name: &str) -> bool {
    keywords::STDLIB_CRATES.contains(&name)
        || ctx.has_dependency(ManifestKind::Cargo, name)
        || ctx.has_dependency(ManifestKind::Cargo, &name.replace('_', "-"))
}

pub(crate) fn detect_flow_inner_with_lookup(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    // First try the chain as-is.
    let direct = detect_flow_inner(file_ctx, ref_ctx);
    if !direct.is_empty() {
        return direct;
    }

    // Let-binding propagation: when the chain root is an Identifier
    // segment, look up the variable's recorded type via field_type
    // (populated for Variable symbols by the Rust extractor). If the
    // type ends with `Client`, rewrite the chain with the type as the
    // new root and rerun the Tonic detector.
    let r = &ref_ctx.extracted_ref;
    let Some(chain) = r.chain.as_ref() else { return Vec::new() };
    let Some(root_seg) = chain.segments.first() else { return Vec::new() };
    if !matches!(root_seg.kind, crate::types::SegmentKind::Identifier) {
        return Vec::new();
    }
    // Resolve the variable in the source symbol's scope and read its type.
    let var_qname = match ref_ctx.source_symbol.scope_path.as_deref() {
        Some(scope) => format!("{}.{}", scope, root_seg.name),
        None => root_seg.name.clone(),
    };
    let type_name = match lookup.field_type_str(&var_qname) {
        Some(t) => t.to_string(),
        None => return Vec::new(),
    };
    // Only rewrite for known-Client patterns.
    if !type_name.ends_with("Client") && !type_name.ends_with("Stub") {
        return Vec::new();
    }
    // Build a substituted chain: replace root identifier with the type +
    // synthetic "new" constructor so the existing Tonic detector matches.
    let mut new_segments = vec![
        crate::types::ChainSegment {
            name: type_name.clone(),
            node_kind: "rewritten_var".to_string(),
            kind: crate::types::SegmentKind::Identifier,
            declared_type: None,
            type_args: vec![],
            optional_chaining: false,
            byte_offset: 0,
                        declared_type_id: None,
            is_call: false,
            type_arg_ids: Vec::new(),
},
        crate::types::ChainSegment {
            name: "new".to_string(),
            node_kind: "rewritten_var".to_string(),
            kind: crate::types::SegmentKind::Property,
            declared_type: None,
            type_args: vec![],
            optional_chaining: false,
            byte_offset: 0,
                        declared_type_id: None,
            is_call: false,
            type_arg_ids: Vec::new(),
},
    ];
    new_segments.extend(chain.segments.iter().skip(1).cloned());
    let rewritten = crate::types::MemberChain { segments: new_segments };
    if let Some(em) = detect_rust_tonic_emission(&rewritten) {
        return vec![em];
    }
    if let Some(em) = detect_rust_reqwest_emission(&rewritten, &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    // HTTP-method route attribute → Consumer HttpCall.
    // Decorators emit a TypeRef with target_name=<verb> and module=URL
    // for actix-web / Rocket `#[get("/x")]` style declarations.
    if r.kind == EdgeKind::TypeRef {
        if let Some(em) = detect_rust_route_attribute_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![em];
        }
        if let Some(em) = detect_rust_tauri_command_attribute(r.target_name.as_str()) {
            // The command name = the function's own symbol name;
            // emission carries a wildcard and the pairer matches it
            // against the TS-side invoke(name) producer.
            return vec![em];
        }
        // async-graphql / juniper attribute markers — `#[Object]`,
        // `#[graphql_object]`, `#[SimpleObject]`, `#[Subscription]`.
        if let Some(em) = detect_rust_async_graphql_attribute(r.target_name.as_str()) {
            return vec![em];
        }
        return Vec::new();
    }

    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }

    // SQLx macros — `sqlx::query!`, `sqlx::query_as!`, `query_scalar!`
    // etc. land as Calls refs with `module = Some("sqlx")` and
    // `target_name` carrying the verb. `call_args` holds the entity
    // name (for query_as) and the SQL string (parsed from the macro
    // body by `extract_macro_string_args`).
    if let Some(em) = detect_rust_sqlx_macro_emission(
        r.target_name.as_str(),
        r.module.as_deref(),
        &r.call_args,
    ) {
        return vec![em];
    }

    let Some(chain) = r.chain.as_ref() else {
        return Vec::new();
    };

    if let Some(em) = detect_rust_axum_route_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_actix_resource_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_reqwest_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_diesel_emission(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_tonic_emission(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_lettre_mailer(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_apalis_bgjob(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_rdkafka_mq(chain) {
        return vec![em];
    }
    if let Some(em) = detect_rust_redis_config_lookup(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_uds_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_rust_axum_ws_consumer(chain) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    // Derive the module path for this file. The Rust extractor sets
    // scope_path on top-level symbols to reflect the module path
    // (e.g., "crate::models" for a symbol in src/models.rs).
    // We take it from the first top-level symbol's scope_path.
    let file_namespace = predicates::extract_module_path(file);

    // Build import entries from EdgeKind::Imports refs.
    // The Rust extractor emits one ref per `use` item brought into scope:
    //   use serde::Deserialize;
    //     → ref { target_name: "Deserialize", module: Some("serde"), kind: Imports }
    //   use crate::models::User;
    //     → ref { target_name: "User", module: Some("crate::models"), kind: Imports }
    //   use std::collections::HashMap;
    //     → ref { target_name: "HashMap", module: Some("std::collections"), kind: Imports }
    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }

        let module_path = r.module.clone().or_else(|| {
            // If no module field, try splitting target_name on "::"
            // e.g., target_name = "serde::Deserialize"
            if r.target_name.contains("::") {
                let (mod_part, _name) = r.target_name.rsplit_once("::")?;
                Some(mod_part.to_string())
            } else {
                None
            }
        });

        // The imported name is the last segment of the path.
        let imported_name = if r.target_name.contains("::") {
            r.target_name
                .rsplit("::")
                .next()
                .unwrap_or(&r.target_name)
                .to_string()
        } else {
            r.target_name.clone()
        };

        // Wildcard import: `use foo::bar::*`
        let is_wildcard = imported_name == "*";

        imports.push(ImportEntry {
            imported_name,
            module_path,
            alias: None,
            is_wildcard,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "rust".to_string(),
        imports,
        file_namespace,
    }
}


// =============================================================================
// LanguageEngineHooks impl + static instance.
// =============================================================================

pub struct RustHooks;

impl crate::type_checker::profile::hooks::LanguageEngineHooks for RustHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        detect_flow_inner_with_lookup(file_ctx, ref_ctx, lookup)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        if let Some(res) = RustResolver.resolve(file_ctx, ref_ctx, lookup) {
            return Some(res);
        }
        (crate::type_checker::core::DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static RUST_HOOKS: RustHooks = RustHooks;