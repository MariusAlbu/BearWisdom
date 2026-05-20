// =============================================================================
// indexer/resolve/rules/kotlin/mod.rs — Kotlin resolution rules
//
// Scope rules for Kotlin:
//
//   1. Scope chain walk: innermost scope → outermost, try {scope}.{target}.
//   2. Same-package resolution: types in the same package are visible without
//      an explicit import (mirrors Java package visibility).
//   3. Exact import resolution: `import com.foo.Bar` → Bar directly visible.
//   4. Wildcard import: `import com.foo.*` → all types in that package visible.
//   5. Fully qualified names: dotted names resolve directly.
//
// Kotlin import model:
//   The Kotlin extractor emits EdgeKind::Imports refs for import statements:
//     import com.foo.Bar    → target_name = "Bar",  module = "com.foo.Bar"
//     import com.foo.*      → target_name = "*",    module = "com.foo"
// =============================================================================


use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::type_checker::chain::{
    self, ChainConfig, NamespaceLookup, identity_normalize,
};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::inheritance;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Kotlin language resolver.
pub struct KotlinResolver;

impl LanguageResolver for KotlinResolver {
    fn language_ids(&self) -> &[&str] {
        &["kotlin"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract the package declaration from Namespace symbols.
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

    fn resolve(
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

        // Bare-name walker lookup. kotlin_stdlib + jdk_src + android_sdk +
        // maven (sources jars) emit real symbols for stdlib functions
        // (apply, let, listOf, ...), JVM types, Android SDK types, and
        // declared Maven/Gradle deps including the Compose test DSL.
        // Bind to ext:-prefixed paths only — internal-name binding is
        // handled by the chain walker and same-file paths below. Skip
        // when the ref has a chain so the chain walker's receiver-type
        // context wins over bare-leaf lookup.
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

        // Chain-aware resolution: walk MemberChain step-by-step following
        // field types. Kotlin is JVM-like: wildcard imports provide namespace context.
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

        // Step 1: Scope chain walk.
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

        // Step 2: Same-package resolution.
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

        // Step 3: Exact import resolution.
        for import in &file_ctx.imports {
            if import.is_wildcard {
                continue;
            }
            // Check both imported_name and alias.
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

        // Step 4: Wildcard import resolution.
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

        // Step 5: Fully qualified name.
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

        // Step 6: Inheritance-chain walk for implicit `this` calls.
        //
        // Kotlin allows bare method calls inside a class body — `myMethod()`
        // is equivalent to `this.myMethod()` and may target a parent class.
        // When Steps 1–5 all miss, walk the inherits_map upward from the
        // enclosing class (scope_chain[1]) trying `{ancestor}.{target}`.
        //
        // Fires for: EdgeKind::Calls, simple (no-dot) name, inside a class.
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

        // Step 7: Bare-name fallback — match any symbol whose `name` field
        // equals the target. Confidence lowered to 0.85 to signal this is
        // a best-guess match, not a scoped/imported resolution.
        //
        // Enables DSL-lambda receivers (Spring MockMvc Kotlin DSL:
        // `mockMvc.andExpect { jsonPath(...) }`), top-level helper calls
        // imported via wildcards the extractor may have lost, and framework-
        // synthesised symbols (spring_stubs, compose_stubs, …) whose
        // qualified path the caller wouldn't know. Mirrors Elixir's step 5.
        //
        // Bare names only (no-dot) for Calls + TypeRef + Instantiates — the
        // dotted-name case already has steps 5/6 above. Imports return early.
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

        // Kotlin bare-name fallback. Continues the cross-language
        // template (PRs 31, 35-40, Lua, Go, Rust). Kotlin extension
        // functions, top-level declarations, and JVM-bridge static
        // imports can call bare names without an explicit module
        // qualifier the engine binds. Gated by `.kt`/`.kts` file
        // extension and `kind_compatible`.
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
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

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");
        match vis {
            "public" | "internal" => true,
            "protected" => true, // allow — full check needs inheritance info
            "private" => &*target.file_path == file_ctx.file_path,
            _ => true,
        }
    }

}

/// Akka `actor.tell(msg, sender)` / `actor.ask(msg)` / `actorRef ! msg`.
/// The `!` infix form is hard to detect through chain refs (operator
/// tokens don't surface in the chain as named segments), so we focus on
/// the method-call shapes that DO. Emits BgJob Producer keyed on the
/// receiving actor name.
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

// ---------------------------------------------------------------------------
// Ktor server routes — `routing { get("/x") { ... } }`
// ---------------------------------------------------------------------------

/// `get("/x") { ... }`, `post("/x") { ... }`, etc. inside a `routing { ... }`
/// block. The block context is not visible here, but the verb + URL shape is
/// distinctive enough: target_name in the verb set, first arg is a URL.
pub(crate) fn detect_kotlin_ktor_route_emission(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    // Ktor WebSocket Consumer: `webSocket("/ws") { ... }`.
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

/// Ktor client `client.get("/x") { ... }` — Producer HttpCall. Chain ends
/// in a verb; chain root is a `client` Identifier.
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

/// Exposed: `Users.select { ... }`, `Users.insert { ... }`,
/// `Users.update(...)`, `Users.deleteWhere { ... }`. Chain root is a
/// PascalCase table object name. DbQuery keyed on the table.
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
    // Avoid pairing on generic util classes — restrict to Exposed-style verbs.
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

/// `<Service>Grpc.newBlockingStub(channel).method(req)` Kotlin shape.
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

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

pub(super) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

    // Import refs — classify the import path itself.
    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);

        // Manifest-driven: check Maven and Gradle group IDs first.
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

    // Gradle version catalog accessors: `libs`, `versions`, `plugins`, and
    // any custom catalog names defined in `gradle/*.versions.toml`.
    // These are DSL properties injected by Gradle — not real Kotlin symbols.
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
        // `plugins` in a `plugins { }` block and `versions` are always
        // Gradle build-script concepts — classify as external regardless
        // of whether we found a catalog file.
        if matches!(root, "plugins" | "versions" | "dev" | "buildSrc") {
            return Some(format!("gradle.catalog.{root}"));
        }
    }

    // Android SDK bare names: Activity, Context, View, Fragment, etc.
    // These are imported from android.* / androidx.* which is already in
    // ALWAYS_EXTERNAL, but they appear as bare names after a wildcard import
    // (e.g. `import android.app.*`). Classify them via the import walk below.

    // Walk imports for a match on this target name.
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

        // Manifest-driven check on import namespace.
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

    // Fully-qualified target.
    if predicates::effective_target_is_external(target, project_ctx) {
        return Some(target.clone());
    }

    None
}

pub(super) fn infer_external_inner_with_lookup(
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

    // Annotation-based: Spring @GetMapping etc. are handled by the
    // ExtractedRoute adapter for Spring-Kotlin. Retrofit `@GET("/x")`
    // attribute Producer.
    if r.kind == EdgeKind::TypeRef {
        if let Some(em) = super::super::java::resolve::detect_retrofit_attribute_emission(
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
        // with target_name = "get"/"post"/etc., no chain. The first arg
        // is the URL string.
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
