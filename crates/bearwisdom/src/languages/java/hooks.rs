// =============================================================================
// languages/java/hooks.rs — JavaHooks impl of LanguageEngineHooks plus the
// concrete JavaResolver (chain-aware, scope-chain, same-package, import,
// wildcard-import, qualified-name, inheritance, bare-name fallback) and the
// external-classifier / flow-detector / file-context helpers it dispatches.
//
// Java import model:
//   The Java extractor emits EdgeKind::Imports refs for import statements:
//     import com.foo.Bar;      → target_name = "Bar",   module = "com.foo.Bar"
//     import com.foo.*;        → target_name = "*",     module = "com.foo"
//
//   Same-package visibility mirrors C# same-namespace: all types in the same
//   package (first N dotted segments of qualified_name) are visible without
//   import.
// =============================================================================

pub(crate) use super::flow_detectors::{
    detect_java_db_query_emission, detect_java_grpc_stub_emission,
    detect_java_http_chain_emission, detect_java_jdbc_template_emission,
    detect_java_jms_kafka_emission, detect_java_mailer_emission,
    detect_java_message_mapping_emission, detect_java_quartz_emission,
    detect_java_redis_template_emission, detect_jpa_query_annotation_emission,
    detect_retrofit_attribute_emission, detect_spring_stereotype_emission,
};
use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::inheritance;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct JavaResolver;

/// Java `ChainConfig` for the unified `resolve_via_chain`.
///
/// Java carries one delta as `ChainExtensions` data — inheritance climbing on a
/// member miss (`extends`/`implements` chains, plus the members_of final
/// fallback the bespoke walker emitted) — and one as `NamespaceLookup::WildcardOnly`
/// for resolving members through wildcard `import com.foo.*` statements
/// (intermediate and final segments). It has no type aliases, no external-qname
/// promotion, no `new X().m()` construction roots, no extension methods, and no
/// ambient-globals root fallback. `enclosing_type_kinds` / `static_type_kinds`
/// match the type kinds the Java extractor emits.
pub(crate) static JAVA_CHAIN_CONFIG: crate::type_checker::chain::ChainConfig =
    crate::type_checker::chain::ChainConfig {
        strategy_prefix: "java",
        normalize_type: crate::type_checker::chain::identity_normalize,
        has_self_ref: true,
        enclosing_type_kinds: &["class", "interface", "enum"],
        static_type_kinds: &["class", "interface", "enum", "type_alias"],
        use_generics: true,
        namespace_lookup: crate::type_checker::chain::NamespaceLookup::WildcardOnly,
        kind_compatible: predicates::kind_compatible,
        extensions: crate::type_checker::chain::ChainExtensions {
            expand_aliases: false,
            walk_inheritance: true,
            promote_external_qname: false,
            root_construction: false,
            extension_method_fallback: false,
            root_fallback: None,
            root_type_access: false,
        },
    };

impl JavaResolver {
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

        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = crate::type_checker::chain::resolve_via_chain(
                &JAVA_CHAIN_CONFIG, chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_same_package",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        for import in &file_ctx.imports {
            if import.is_wildcard {
                continue;
            }
            if import.imported_name == effective_target {
                if let Some(module) = &import.module_path {
                    if let Some(sym) = lookup.by_qualified_name(module) {
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "java_import",
                                resolved_yield_type: None,
                                flow_emit: None,
                            });
                        }
                    }
                }
            }
        }

        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            if let Some(module) = &import.module_path {
                let candidate = format!("{module}.{effective_target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "java_wildcard_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }

        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "java_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Inheritance-chain walk for implicit `this` calls.
        //
        // Java and Groovy both allow bare method calls inside a class body —
        // `myMethod()` means `this.myMethod()` and can target a parent class.
        if edge_kind == EdgeKind::Calls && !effective_target.contains('.') {
            if let Some(calling_class) =
                inheritance::enclosing_class_from_scope(&ref_ctx.source_symbol.qualified_name, lookup)
            {
                if let Some(res) = inheritance::resolve_via_inheritance(
                    calling_class,
                    effective_target,
                    edge_kind,
                    file_ctx,
                    ref_ctx,
                    lookup,
                    predicates::kind_compatible,
                    |fc, rc, sym| self.is_visible(fc, rc, sym),
                    "java_inherited_method",
                ) {
                    return Some(res);
                }
            }
        }

        // Bare-name fallback. Spring fluent APIs, Stream / Optional
        // methods, and AssertJ matchers leave the chain walker without
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

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        if let Some(ctx) = project_ctx {
            for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&kind) {
                    if manifest.dependencies.iter().any(|group_id| {
                        import_path == group_id
                            || import_path.starts_with(group_id.as_str())
                                && import_path.as_bytes().get(group_id.len()) == Some(&b'.')
                    }) {
                        return Some(import_path.to_string());
                    }
                }
            }
        }
        if predicates::is_external_java_namespace(import_path, project_ctx) {
            return Some(import_path.to_string());
        }
        if let Some(lookup) = lookup {
            if !lookup.has_in_namespace(import_path) {
                return Some(import_path.to_string());
            }
        }
        return None;
    }

    for import in &file_ctx.imports {
        let ns = import.module_path.as_deref().unwrap_or("");
        if ns.is_empty() {
            continue;
        }
        if !import.is_wildcard && import.imported_name != *target {
            continue;
        }
        if let Some(ctx) = project_ctx {
            for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&kind) {
                    if manifest.dependencies.iter().any(|group_id| {
                        ns == group_id
                            || ns.starts_with(group_id.as_str())
                                && ns.as_bytes().get(group_id.len()) == Some(&b'.')
                    }) {
                        return Some(ns.to_string());
                    }
                }
            }
        }
        if predicates::is_external_java_namespace(ns, project_ctx) {
            return Some(ns.to_string());
        }
        if let Some(lookup) = lookup {
            if !lookup.has_in_namespace(ns) {
                return Some(ns.to_string());
            }
        }
    }

    if predicates::effective_target_is_external(target, project_ctx) {
        return Some(target.clone());
    }

    None
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    if r.kind == EdgeKind::TypeRef {
        if let Some(emission) = detect_jpa_query_annotation_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![emission];
        }
        if let Some(emission) = detect_retrofit_attribute_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![emission];
        }
        if let Some(emission) = detect_java_message_mapping_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![emission];
        }
        if let Some(emission) = detect_spring_stereotype_emission(r.target_name.as_str()) {
            return vec![emission];
        }
        return Vec::new();
    }

    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let Some(chain) = r.chain.as_ref() else { return Vec::new(); };
    if let Some(emission) = detect_java_http_chain_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_java_db_query_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_java_jdbc_template_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_java_grpc_stub_emission(chain) {
        return vec![emission];
    }
    if let Some(emission) = detect_java_mailer_emission(chain) {
        return vec![emission];
    }
    if let Some(emission) = detect_java_quartz_emission(chain) {
        return vec![emission];
    }
    if let Some(emission) = detect_java_jms_kafka_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_java_redis_template_emission(chain, &r.call_args) {
        return vec![emission];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    let file_namespace = file.symbols.iter().find_map(|sym| {
        if sym.kind == crate::types::SymbolKind::Namespace {
            Some(sym.qualified_name.clone())
        } else {
            None
        }
    });

    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        let module = r.module.as_deref().unwrap_or(&r.target_name);
        let is_wildcard = r.target_name == "*";

        if is_wildcard {
            imports.push(ImportEntry {
                imported_name: String::new(),
                module_path: Some(module.to_string()),
                alias: None,
                is_wildcard: true,
            });
        } else {
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module.to_string()),
                alias: None,
                is_wildcard: false,
            });
        }
    }

    FileContext {
        file_path: file.path.clone(),
        language: "java".to_string(),
        imports,
        file_namespace,
    }
}

pub struct JavaHooks;

impl LanguageEngineHooks for JavaHooks {
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
        let _ = lookup;
        detect_flow_inner(file_ctx, ref_ctx)
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
        if let Some(res) = JavaResolver.resolve(file_ctx, ref_ctx, lookup) {
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

pub static JAVA_HOOKS: JavaHooks = JavaHooks;
