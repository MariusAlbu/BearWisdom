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
    intern_yield_type, ChainMiss, FileContext, ImportEntry, RefContext, Resolution, SymbolInfo,
    SymbolLookup,
};
use crate::type_checker::chain::simple_yield_type;
use crate::type_checker::inheritance;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

pub struct JavaResolver;

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

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Bare-name walker lookup. jdk_src + maven (sources jars) emit real
        // symbols for java.lang types (String, Integer, Object), exception
        // hierarchy, Object methods, Stream / Collection / List APIs, etc.
        // ext:-only filter so chain walker / scope / same-package paths
        // still win for project symbols. Skip when the ref has a chain so
        // the chain walker's receiver-type context wins.
        if ref_ctx.extracted_ref.chain.is_none() && !target.contains('.') {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "java_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = walk_java_chain(chain_val, edge_kind, file_ctx, ref_ctx, lookup) {
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
                inheritance::enclosing_class_from_scope(&ref_ctx.scope_chain)
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
        // a usable declared type at the leaf segment. The leaf method
        // IS in the externals index — it just can't be bound by chain
        // walking alone. File-extension gate prevents cross-language
        // collisions.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !effective_target.contains('.')
        {
            for sym in lookup.by_name(effective_target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_java = path.ends_with(".java")
                    || path.ends_with(".jar")
                    || path.starts_with("ext:java:")
                    || path.starts_with("ext:idx:");
                if !is_java {
                    continue;
                }
                if !self.is_visible(file_ctx, ref_ctx, sym) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "java_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        None
    }

    pub(crate) fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");
        match vis {
            "public" => true,
            "package" => {
                let target_pkg = predicates::first_segment(&target.file_path);
                let source_pkg = predicates::first_segment(&file_ctx.file_path);
                target_pkg == source_pkg
            }
            "protected" => true,
            "private" => &*target.file_path == file_ctx.file_path,
            _ => true,
        }
    }
}

/// Java chain walker.
pub(crate) fn walk_java_chain(
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
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            if let Some(local_type) = lookup.local_type(name) {
                Some(local_type)
            } else {
                // Is it a known class/type? (static access: `ClassName.method()`)
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "class" | "interface" | "enum" | "type_alias"
                    )
                });
                if is_type {
                    Some(name.clone())
                } else {
                    // Is it a field on the enclosing class?
                    let mut found = None;
                    for scope in &ref_ctx.scope_chain {
                        let field_qname = format!("{scope}.{name}");
                        if let Some(type_name) = lookup.field_type_str(&field_qname) {
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

    // Phase 2: Walk intermediate segments, following field types or return types.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_str(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }
        if let Some(next_type) = lookup.return_type_str(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }

        // Try via import namespaces: {namespace}.{current_type}.{field}
        let mut found = false;
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(module) = &import.module_path {
                    let qualified_member = format!("{module}.{member_qname}");
                    if let Some(next_type) = lookup.field_type_str(&qualified_member) {
                        current_type = next_type.to_string();
                        found = true;
                        break;
                    }
                    if let Some(next_type) = lookup.return_type_str(&qualified_member) {
                        current_type = next_type.to_string();
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

    // Phase 3: Resolve the final segment on the resolved type.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "java_chain_resolution",
                resolved_yield_type: intern_yield_type(simple_yield_type(sym, lookup), lookup),
                flow_emit: None,
            });
        }
    }

    // Try via wildcard imports: {namespace}.{resolved_type}.{method}
    for import in &file_ctx.imports {
        if import.is_wildcard {
            if let Some(module) = &import.module_path {
                let ns_candidate = format!("{module}.{candidate}");
                if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "java_chain_resolution",
                            resolved_yield_type: intern_yield_type(
                                simple_yield_type(sym, lookup),
                                lookup,
                            ),
                            flow_emit: None,
                        });
                    }
                }
            }
        }
    }

    // Members scoped to the resolved type.
    for sym in lookup.members_of(&current_type) {
        if sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "java_chain_resolution",
                resolved_yield_type: intern_yield_type(simple_yield_type(sym, lookup), lookup),
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

/// Find the enclosing class/interface from the scope chain.
///
/// Java scope_chain: `["com.example.OrderService.create", "com.example.OrderService", "com.example"]`
/// We want "com.example.OrderService".
pub(crate) fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "interface" | "enum") {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: second-to-last is often the class.
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
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
        JavaResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static JAVA_HOOKS: JavaHooks = JavaHooks;
