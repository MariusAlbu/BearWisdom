// =============================================================================
// languages/kotlin/hooks.rs — KotlinHooks impl plus the concrete
// KotlinResolver (chain via generic ChainConfig walker over class/interface/
// object enclosing types with WildcardOnly NamespaceLookup, synthetic-global
// preference for kotlin_stdlib / JDK / android SDK / Maven, scope-chain
// walk, same-package, exact + wildcard imports with alias support, fully-
// qualified names, inheritance via implicit `this`, by-name lookup for
// DSL-lambda receivers and synthesized stubs, `.kt`/`.kts` bare-name
// fallback for extension functions and top-level declarations) plus 5 flow
// detectors (Akka tell/ask, Ktor server routes including WebSocket,
// Ktor client, Exposed ORM, gRPC stub/CoroutineStub) plus external
// classifier with Gradle version-catalog accessors and namespace-negative
// fallback plus build_file_context.
// =============================================================================

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::chain::{
    self, ChainConfig, NamespaceLookup, identity_normalize,
};
use crate::type_checker::inheritance;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct KotlinResolver;

impl KotlinResolver {
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

        // Synthetic-global lookup. kotlin_stdlib + jdk_src + android_sdk +
        // maven (sources jars) emit real symbols for stdlib functions
        // (apply, let, listOf, …), JVM types, Android SDK types, and
        // declared Maven/Gradle deps including the Compose test DSL.
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
                    strategy: "kotlin_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            let config = ChainConfig {
                strategy_prefix: "kotlin",
                normalize_type: identity_normalize,
                has_self_ref: true,
                enclosing_type_kinds: &["class", "interface", "object"],
                static_type_kinds: &["class", "interface", "enum", "type_alias", "object"],
                use_generics: true,
                namespace_lookup: NamespaceLookup::WildcardOnly,
                kind_compatible: predicates::kind_compatible,
            };
            if let Some(res) = chain::resolve_via_chain(
                &config, chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
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
                        strategy: "kotlin_scope_chain",
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
                        strategy: "kotlin_same_package",
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
            let name_match = import.imported_name == effective_target
                || import.alias.as_deref() == Some(effective_target);
            if !name_match {
                continue;
            }
            if let Some(module) = &import.module_path {
                if let Some(sym) = lookup.by_qualified_name(module) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "kotlin_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
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
                            strategy: "kotlin_wildcard_import",
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
                        strategy: "kotlin_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Inheritance walk for implicit `this` calls.
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
                    "kotlin_inherited_method",
                ) {
                    return Some(res);
                }
            }
        }

        // By-name lookup. Enables DSL-lambda receivers (Spring MockMvc Kotlin
        // DSL: `mockMvc.andExpect { jsonPath(...) }`), top-level helpers
        // imported via wildcards the extractor may have lost, and framework-
        // synthesised symbols (spring_stubs, compose_stubs, …) whose
        // qualified path the caller wouldn't know.
        if !effective_target.contains('.')
            && matches!(
                edge_kind,
                EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates
            )
        {
            for sym in lookup.by_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.85,
                        strategy: "kotlin_by_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // `.kt`/`.kts`-gated bare-name fallback for extension functions,
        // top-level declarations, and JVM-bridge static imports.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_kotlin = path.ends_with(".kt") || path.ends_with(".kts");
                if !is_kotlin {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "kotlin_bare_name",
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
            "public" | "internal" => true,
            "protected" => true,
            "private" => &*target.file_path == file_ctx.file_path,
            _ => true,
        }
    }
}

// Akka `actor.tell(msg, sender)` / `actor.ask(msg)` / `actorRef ! msg`.
// The `!` infix form is hard to detect through chain refs (operator tokens
// don't surface as named segments), so we focus on the method-call shapes
// that do.
pub(crate) fn detect_kotlin_akka_tell_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    let is_actor_root = root.ends_with("Actor")
        || root.ends_with("Ref")
        || matches!(root, "actor" | "actorRef" | "ref");
    if !is_actor_root {
        return None;
    }
    if !matches!(leaf, "tell" | "ask" | "forward") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: format!("kt.akka.{}", root),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

// Ktor `routing { get("/x") { ... } }` server route — Consumer.
// `webSocket("/ws") { ... }` — WebSocket Consumer.
pub(crate) fn detect_kotlin_ktor_route_emission(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    if target_name == "webSocket" {
        let url = call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) if s.starts_with('/') => Some(s.as_str()),
            _ => None,
        })?;
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::WebSocket,
            name: crate::connectors::url_pattern::normalize(url),
            role: ChannelRole::Consumer,
            method: None,
            streaming: None,
        });
    }

    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        _ => return None,
    };
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.as_str(),
        _ => return None,
    };
    if !url.starts_with('/') {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(method),
        streaming: None,
    })
}

// Ktor client `client.get("/x") { ... }` — Producer HttpCall.
pub(crate) fn detect_kotlin_ktor_client_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    if chain.segments.len() < 2 {
        return None;
    }
    let leaf = chain.segments.last()?.name.as_str();
    let method = match leaf {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        _ => return None,
    };
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.as_str(),
        _ => return None,
    };
    if !(url.starts_with('/') || url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Producer,
        method: Some(method),
        streaming: None,
    })
}

// Exposed ORM: `Users.select { ... }`, `Users.insert { ... }`, etc.
// Chain root is PascalCase table object; leaf op classifies.
pub(crate) fn detect_kotlin_exposed_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !root.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
        return None;
    }
    let op = match leaf {
        "select" | "selectAll" | "selectBatched" | "find" | "findById" | "all" | "count" => DbQueryOp::Select,
        "insert" | "insertAndGetId" | "batchInsert" => DbQueryOp::Insert,
        "update" | "batchUpdate" => DbQueryOp::Update,
        "delete" | "deleteWhere" | "deleteAll" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("kt.{}", root),
        operation: op,
    })
}

// `<Service>Grpc.newBlockingStub(channel).method(req)` Kotlin shape.
pub(crate) fn detect_kotlin_grpc_stub_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !(root.ends_with("Grpc") || root.ends_with("GrpcKt") || root.ends_with("CoroutineStub")) {
        return None;
    }
    let ctor = segs[1].name.as_str();
    if !matches!(ctor, "newBlockingStub" | "newFutureStub" | "newStub" | "newCoroutineStub") {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    if matches!(leaf, "newBlockingStub" | "newFutureStub" | "newStub" | "newCoroutineStub") {
        return None;
    }
    let service = root
        .strip_suffix("Grpc")
        .or_else(|| root.strip_suffix("GrpcKt"))
        .or_else(|| root.strip_suffix("CoroutineStub"))
        .unwrap_or(root);
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!("{}.{}", service, leaf),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
    })
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
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
                                && import_path.as_bytes().get(group_id.len())
                                    == Some(&b'.')
                    }) {
                        return Some(import_path.to_string());
                    }
                }
            }
        }

        if predicates::is_external_kotlin_namespace(import_path, project_ctx) {
            return Some(import_path.to_string());
        }
        return None;
    }

    // Gradle version-catalog accessors (`libs`, `versions`, `plugins`, plus
    // custom catalog names from `gradle/*.versions.toml`). These are DSL
    // properties injected by Gradle, not real Kotlin symbols.
    {
        let root = target.split('.').next().unwrap_or(target);
        if let Some(ctx) = project_ctx {
            let catalog_names = ctx
                .plugin_state
                .get::<crate::ecosystem::manifest::gradle::GradleCatalogNames>()
                .map(|c| c.0.as_slice())
                .unwrap_or_default();
            if catalog_names.iter().any(|n| n == root) {
                return Some(format!("gradle.catalog.{root}"));
            }
        }
        if matches!(root, "plugins" | "versions" | "dev" | "buildSrc") {
            return Some(format!("gradle.catalog.{root}"));
        }
    }

    for import in &file_ctx.imports {
        let ns = import.module_path.as_deref().unwrap_or("");
        if ns.is_empty() {
            continue;
        }
        if !import.is_wildcard && import.imported_name != *target
            && import.alias.as_deref() != Some(target.as_str())
        {
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

        if predicates::is_external_kotlin_namespace(ns, project_ctx) {
            return Some(ns.to_string());
        }
    }

    if predicates::effective_target_is_external(target, project_ctx) {
        return Some(target.clone());
    }

    None
}

pub(crate) fn infer_external_inner_with_lookup(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    if let Some(ns) = infer_external_inner(file_ctx, ref_ctx, project_ctx) {
        return Some(ns);
    }
    // Structural fallback: imported namespace with no internal symbols
    // → external. Catches transitive Maven / Gradle deps and platform
    // SDKs (Apple Foundation on Native, Servlet on JVM, etc.) that
    // aren't on the manifest's group-id prefix list.
    let target = &ref_ctx.extracted_ref.target_name;
    for import in &file_ctx.imports {
        let Some(ns) = import.module_path.as_deref() else { continue };
        if ns.is_empty() {
            continue;
        }
        if !import.is_wildcard
            && import.imported_name != *target
            && import.alias.as_deref() != Some(target.as_str())
        {
            continue;
        }
        if !lookup.has_in_namespace(ns) {
            return Some(ns.to_string());
        }
    }
    None
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    // Spring @GetMapping etc. are handled by the ExtractedRoute adapter for
    // Spring-Kotlin. Retrofit `@GET("/x")` attribute Producer routes through
    // the Java hook's attribute detector (shared with Kotlin since both
    // target the JVM).
    if r.kind == EdgeKind::TypeRef {
        if let Some(em) = super::super::java::hooks::detect_retrofit_attribute_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![em];
        }
        return Vec::new();
    }
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let Some(chain) = r.chain.as_ref() else {
        // Ktor `routing { get("/x") { ... } }` lands as a bare Calls ref
        // with target_name = "get"/"post"/etc., no chain.
        if let Some(em) = detect_kotlin_ktor_route_emission(
            r.target_name.as_str(),
            &r.call_args,
        ) {
            return vec![em];
        }
        return Vec::new();
    };
    if let Some(em) = detect_kotlin_ktor_client_emission(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_kotlin_exposed_emission(chain) {
        return vec![em];
    }
    if let Some(em) = detect_kotlin_grpc_stub_emission(chain) {
        return vec![em];
    }
    if let Some(em) = detect_kotlin_akka_tell_emission(chain) {
        return vec![em];
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
        language: "kotlin".to_string(),
        imports,
        file_namespace,
    }
}

pub struct KotlinHooks;

impl LanguageEngineHooks for KotlinHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner_with_lookup(file_ctx, ref_ctx, project_ctx, lookup)
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
        KotlinResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static KOTLIN_HOOKS: KotlinHooks = KotlinHooks;
