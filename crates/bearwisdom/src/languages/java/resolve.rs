// =============================================================================
// indexer/resolve/rules/java/mod.rs — Java resolution rules
//
// Scope rules for Java:
//
//   1. Chain-aware resolution: walk MemberChain following field/return types.
//   2. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   3. Same-package resolution: types in the same package are visible without
//      explicit import (Java package visibility).
//   4. Import resolution: `import com.foo.Bar;` makes Bar directly visible.
//   5. Wildcard import: `import com.foo.*;` makes all types in that package visible.
//   6. Fully qualified names: dotted names resolve directly.
//
// Java import model:
//   The Java extractor emits EdgeKind::Imports refs for import statements:
//     import com.foo.Bar;      → target_name = "Bar",   module = "com.foo.Bar"
//     import com.foo.*;        → target_name = "*",      module = "com.foo"
//
//   Same-package visibility mirrors C# same-namespace: all types in the same
//   package (first N dotted segments of qualified_name) are visible without import.
//
// Adding new Java features:
//   - New import forms (e.g., static imports) → add to build_file_context.
//   - New scope forms → update scope_path in the extractor; scope chain handles them.
// =============================================================================


use super::{predicates, type_checker::JavaChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::inheritance;
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Java language resolver.
pub struct JavaResolver;

impl LanguageResolver for JavaResolver {
    fn language_ids(&self) -> &[&str] {
        &["java", "groovy"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract the package declaration from symbols.
        // Java extractor emits a Namespace symbol whose qualified_name is the package.
        let file_namespace = file.symbols.iter().find_map(|sym| {
            if sym.kind == crate::types::SymbolKind::Namespace {
                Some(sym.qualified_name.clone())
            } else {
                None
            }
        });

        // Extract per-file import directives from EdgeKind::Imports refs.
        // Java extractor emits:
        //   import com.foo.Bar;   → target_name = "Bar", module = "com.foo.Bar"
        //   import com.foo.*;     → target_name = "*",   module = "com.foo"
        //   import static ...;    → skipped (captured as Calls/TypeRef by extractor)
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module = r.module.as_deref().unwrap_or(&r.target_name);
            let is_wildcard = r.target_name == "*";

            if is_wildcard {
                // `import com.foo.*;` — all public types in the package visible.
                imports.push(ImportEntry {
                    imported_name: String::new(),
                    module_path: Some(module.to_string()),
                    alias: None,
                    is_wildcard: true,
                });
            } else {
                // `import com.foo.Bar;` — exact type import.
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

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Skip import refs themselves — they're not symbol references.
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

        // Chain-aware resolution: dispatch to JavaChecker.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = JavaChecker.resolve_chain(
                chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // Normalize: strip `this.` prefix for member access on the current class.
        let effective_target = target.strip_prefix("this.").unwrap_or(target);

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["com.example.MyClass.myMethod", "com.example.MyClass", "com.example"]
        // Try "com.example.MyClass.myMethod.Target", "com.example.MyClass.Target", etc.
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

        // Step 2: Same-package resolution.
        // In Java, types in the same package are visible without an explicit import.
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

        // Step 3: Exact import resolution.
        // `import com.foo.Bar;` → target "Bar" resolves to "com.foo.Bar"
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

        // Step 4: Wildcard import resolution.
        // `import com.foo.*;` → try "com.foo.{target}"
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

        // Step 5: Fully qualified name (target contains dots).
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

        // Step 6: Inheritance-chain walk for implicit `this` calls.
        //
        // Java and Groovy both allow bare method calls inside a class body —
        // `myMethod()` means `this.myMethod()` and can target a parent class.
        // When Steps 1–5 all miss, walk `inherits_map` upward from the
        // enclosing class (scope_chain[1]) trying `{ancestor}.{target}` at
        // each level (depth ≤ 10 to guard against cycles).
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
                    "java_inherited_method",
                ) {
                    return Some(res);
                }
            }
        }

        // Java bare-name fallback. Counterpart to the SCSS / Bash /
        // Python `<lang>_bare_name` resolver steps. Java chain refs
        // through Spring fluent APIs (`mockMvc.perform(...).andExpect(
        // model().attributeHasErrors(...))`), Stream / Optional methods,
        // and AssertJ matchers leave the chain walker without a usable
        // declared type by the leaf segment. The leaf method itself IS
        // in the externals index — it just can't be bound by chain
        // walking alone.
        //
        // Index-wide `by_name` lookup gated by `.java` file path and
        // `kind_compatible`. Cross-language collisions can't leak
        // because the file-extension filter excludes Python / TS /
        // etc. defining identically-named methods.
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
                // Honor Java visibility — private methods aren't reachable
                // across files even by bare name. `is_visible` runs the
                // same checks as the deterministic resolution paths so a
                // private cross-file method stays unresolved.
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

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, None)
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");
        match vis {
            "public" => true,
            // package-private (no modifier): visible within the same package.
            "package" => {
                // Approximate: same top-level package prefix.
                let target_pkg = predicates::first_segment(&target.file_path);
                let source_pkg = predicates::first_segment(&file_ctx.file_path);
                target_pkg == source_pkg
            }
            "protected" => {
                // Accessible from same package or subclasses.
                // Approximate: allow (full check requires inheritance info).
                true
            }
            "private" => {
                // Only visible within the same file (same class declaration).
                &*target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }

    fn detect_flow_emission(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let r = &ref_ctx.extracted_ref;

        // Annotation-based detection — Spring Data `@Query`, Retrofit
        // `@GET("/x")` Producer. Spring controller routes
        // (`@GetMapping` / `@PostMapping` / `@RequestMapping`) are already
        // surfaced as Consumer HttpCall via the generic ExtractedRoute
        // adapter, so we don't double-emit those here.
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

        // Chain-call detection — RestTemplate / WebClient / OkHttp Producer,
        // EntityManager JPA + JdbcTemplate DbQuery, and gRPC Stub RpcCall.
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
}

/// Java Quartz `scheduler.scheduleJob(...)`, Spring `@Scheduled`-style
/// programmatic registration. Single-ended BgJob Producer.
pub(crate) fn detect_java_quartz_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "scheduler" | "jobScheduler" | "quartzScheduler") {
        return None;
    }
    if !matches!(leaf, "scheduleJob" | "scheduleJobs") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: "java.quartz".to_string(),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

/// JMS `producer.send(message)` and Kafka `kafkaTemplate.send(topic, payload)`.
/// Captures the topic (Kafka) when it's a string literal.
pub(crate) fn detect_java_jms_kafka_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if leaf != "send" {
        return None;
    }
    // Spring KafkaTemplate — topic from first string arg.
    if matches!(root, "kafkaTemplate" | "kafkaProducer") {
        let topic = call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        });
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: topic.unwrap_or_else(|| "java.kafka".to_string()),
            role: ChannelRole::Producer,
            method: None,
        streaming: None,
        });
    }
    // JMS — `producer.send(message)`.
    if matches!(root, "jmsTemplate" | "producer" | "messageProducer") {
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::MessageQueue,
            name: "java.jms".to_string(),
            role: ChannelRole::Producer,
            method: None,
        streaming: None,
        });
    }
    None
}

/// Spring STOMP `@MessageMapping("/x")` or `@SubscribeMapping("/x")` on a
/// handler method → WebSocket Consumer keyed on the destination.
/// Match a Spring stereotype annotation (`@Service`, `@Repository`,
/// `@Component`, `@RestController`, `@Controller`) on the class TypeRef.
/// Emits a single-ended `DiBinding` with `container = "spring"` so the
/// architecture overview clusters Spring-managed beans. Pair-keyed
/// matching to the implemented interface (via `implements` edges) needs
/// cross-ref state the resolver doesn't carry; the single-ended marker
/// still clusters the bean as a DI participant.
pub(crate) fn detect_spring_stereotype_emission(
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    if !matches!(
        target,
        "Service" | "Repository" | "Component" | "RestController" | "Controller"
    ) {
        return None;
    }
    Some(FlowEmission::DiBinding {
        service_symbol_id: 0,
        container: Some("spring".to_string()),
    })
}

pub(crate) fn detect_java_message_mapping_emission(
    target: &str,
    module: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    // Spring STOMP routing annotations + Jakarta WebSocket / Tyrus
    // lifecycle annotations. `@ServerEndpoint("/ws")` is the most
    // useful one for pairing — it carries the WS path. The lifecycle
    // ones (`@OnOpen`, `@OnMessage`, `@OnClose`, `@OnError`) mark
    // methods on an already-declared endpoint class; they emit a
    // single-ended marker so the class clusters with WS endpoints.
    if !matches!(
        target,
        "MessageMapping"
            | "SubscribeMapping"
            | "OnMessage"
            | "OnOpen"
            | "OnClose"
            | "OnError"
            | "ServerEndpoint"
            | "ClientEndpoint"
    ) {
        return None;
    }
    let dest = module.unwrap_or("");
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: if dest.is_empty() { "java.ws".to_string() } else { dest.to_string() },
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

/// Spring Data Redis `redisTemplate.opsForValue().get(key)` → ConfigLookup.
pub(crate) fn detect_java_redis_template_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;
    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "redisTemplate" | "stringRedisTemplate") {
        return None;
    }
    if leaf != "get" {
        return None;
    }
    // Look for `opsForValue` (or `opsForHash` / `opsForSet`) in the chain.
    if !chain
        .segments
        .iter()
        .any(|s| s.name.starts_with("opsFor"))
    {
        return None;
    }
    let key = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })?;
    Some(FlowEmission::ConfigLookup {
        key: format!("redis:{}", key),
    })
}

pub(crate) fn detect_java_mailer_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    // JavaMailSender.send / mailSender.send.
    if !matches!(root, "mailSender" | "javaMailSender" | "emailService" | "mailService") {
        return None;
    }
    if !matches!(leaf, "send" | "sendAsync") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("java.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// HTTP Producer detection — RestTemplate / WebClient / OkHttp
// ---------------------------------------------------------------------------

/// Recognise Java HTTP-client chain calls and emit Producer
/// `NamedChannel { kind: HttpCall, .. }` keyed on the first string-literal arg.
///
/// Shapes handled:
/// - **RestTemplate**: `<rt>.getForObject("/x", ...)`, `.getForEntity(...)`,
///   `.postForObject(...)`, `.postForEntity(...)`, `.put(...)`, `.delete(...)`,
///   `.patchForObject(...)`, `.headForHeaders(...)`, `.exchange(...)`.
/// - **WebClient (fluent)**: `<wc>.get().uri("/x").retrieve()...` —
///   detect via leaf `uri` AND chain containing one of `get`/`post`/`put`/
///   `patch`/`delete`/`head`/`options`/`method` earlier in the chain.
/// - **OkHttp**: `<builder>.url("...")` — emit Any-method (verb is on a
///   separate `.method(...)` / `.get()` / `.post()` builder call we don't
///   correlate to the url() call statically).
pub(crate) fn detect_java_http_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();

    // First arg as a string literal — required for a pairing key.
    let first_string = match call_args.first()? {
        CallArg::StringLit(s) => Some(s.clone()),
        _ => None,
    };

    // RestTemplate single-call verb methods.
    if let Some(method) = parse_resttemplate_verb(leaf) {
        let url = first_string?;
        if url.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url),
            role: ChannelRole::Producer,
            method: Some(method),
        streaming: None,
        });
    }

    // WebClient `.uri('/x')` — verb comes from a separate `.get()`/`.post()`
    // segment earlier in the chain. Scan chain segments for the verb token.
    if leaf == "uri" {
        let method = chain
            .segments
            .iter()
            .find_map(|s| parse_webclient_verb(s.name.as_str()))
            .unwrap_or(HttpMethod::Any);
        let url = first_string?;
        if url.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url),
            role: ChannelRole::Producer,
            method: Some(method),
        streaming: None,
        });
    }

    // OkHttp `.url("https://api.example.com/x")` — verb on a separate
    // builder method, so emit Any here.
    if leaf == "url" {
        let url = first_string?;
        if url.is_empty() {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(&url),
            role: ChannelRole::Producer,
            method: Some(HttpMethod::Any),
        streaming: None,
        });
    }

    None
}

fn parse_resttemplate_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "getForObject" | "getForEntity" => HttpMethod::Get,
        "postForObject" | "postForEntity" | "postForLocation" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patchForObject" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "headForHeaders" => HttpMethod::Head,
        "optionsForAllow" => HttpMethod::Options,
        "exchange" | "execute" => HttpMethod::Any,
        _ => return None,
    })
}

fn parse_webclient_verb(name: &str) -> Option<crate::indexer::resolve::flow_emit::HttpMethod> {
    use crate::indexer::resolve::flow_emit::HttpMethod;
    Some(match name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        "method" => HttpMethod::Any,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// JPA / Spring Data — DbQuery emission
// ---------------------------------------------------------------------------

/// Recognise JPA EntityManager chain calls and emit a `DbQuery` keyed on
/// the entity class from the first arg (`Entity.class`). Shapes handled:
/// - `entityManager.find(Entity.class, id)`
/// - `entityManager.getReference(Entity.class, id)`
/// - `entityManager.persist(entityInstance)` — entity name unknown, skipped.
/// - `entityManager.remove(...)` / `merge(...)` — same, skipped.
/// - `entityManager.createQuery("FROM Entity", Entity.class)` — emit on
///   the second class-literal arg when present.
///
/// Spring Data derived methods (`findByName`, `existsByEmail`) and
/// generic-typed repository calls are deferred — they require resolving
/// the repository's `<T>` type parameter to a model class, which is not
/// statically recoverable from the chain alone.
pub(crate) fn detect_java_db_query_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();

    // EntityManager method names that take Entity.class as the first arg.
    let op = match leaf {
        "find" | "getReference" => DbQueryOp::Select,
        "createQuery" | "createNativeQuery" => DbQueryOp::Select,
        _ => return None,
    };
    if !is_entity_manager_root(root) {
        return None;
    }

    // First-arg shape varies. `find(Entity.class, id)` → class literal first.
    // `createQuery("FROM ...", Entity.class)` → string first, class second.
    // Pick the class-literal arg.
    let entity = call_args.iter().find_map(|a| match a {
        CallArg::Ident(name) if is_pascal_case_first_java(name) => Some(name.clone()),
        _ => None,
    })?;

    Some(FlowEmission::DbQuery {
        entity_name: format!("java.{}", entity),
        operation: op,
    })
}

fn is_entity_manager_root(name: &str) -> bool {
    matches!(
        name,
        "entityManager"
            | "em"
            | "entityManagerFactory"
            | "session" // Hibernate Session has similar API
            | "sessionFactory"
    )
}

fn is_pascal_case_first_java(name: &str) -> bool {
    name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// `@Query("...")` annotation → DbQuery emission
// ---------------------------------------------------------------------------

/// Spring Data `@Query("SELECT u FROM User u WHERE ...")` annotation on a
/// repository interface method. Emit `DbQuery` keyed on the entity name
/// parsed from the JPQL `FROM <Entity>` clause. Native queries use a SQL
/// table name we can't reliably map to a Java class, so for `@Query` we
/// only handle JPQL where the FROM clause is a class name (PascalCase).
pub(crate) fn detect_jpa_query_annotation_emission(
    annotation_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if annotation_name != "Query" {
        return None;
    }
    let sql = first_arg?.trim();
    if sql.is_empty() {
        return None;
    }
    let upper = sql.to_ascii_uppercase();
    let from_idx = upper.find(" FROM ").or_else(|| upper.find("FROM "))?;
    let after_from = &sql[from_idx + 5..];
    let entity = after_from
        .split_whitespace()
        .next()?
        .trim_end_matches(',')
        .trim_end_matches(';');
    if !is_pascal_case_first_java(entity) {
        return None;
    }
    let op = if upper.starts_with("SELECT") || upper.contains(" SELECT ") {
        DbQueryOp::Select
    } else if upper.starts_with("UPDATE") || upper.contains(" UPDATE ") {
        DbQueryOp::Update
    } else if upper.starts_with("DELETE") || upper.contains(" DELETE ") {
        DbQueryOp::Delete
    } else if upper.starts_with("INSERT") || upper.contains(" INSERT ") {
        DbQueryOp::Insert
    } else {
        DbQueryOp::Select
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("java.{}", entity),
        operation: op,
    })
}

fn infer_external_inner(
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
            // No internal symbols under this fully-qualified namespace
            // → external. Catches transitive deps and stdlib packages
            // not in `is_external_java_namespace`'s known prefix list.
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

// ---------------------------------------------------------------------------
// JdbcTemplate — DbQuery emission
// ---------------------------------------------------------------------------

/// Recognise JdbcTemplate / NamedParameterJdbcTemplate chains:
/// - `jdbcTemplate.query("SELECT ...", rowMapper)`,
///   `queryForObject(...)`, `queryForList(...)`, `queryForMap(...)`,
///   `queryForRowSet(...)`.
/// - `jdbcTemplate.update(...)`, `batchUpdate(...)`, `execute(...)`.
///
/// First arg as a SQL string yields the table; the verb is the chain leaf.
pub(crate) fn detect_java_jdbc_template_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !is_jdbc_template_root(root) {
        return None;
    }
    let leaf_op = match leaf {
        "query" | "queryForObject" | "queryForList" | "queryForMap"
        | "queryForRowSet" | "queryForStream" => DbQueryOp::Select,
        "update" | "batchUpdate" => DbQueryOp::Update,
        "execute" => DbQueryOp::Other,
        _ => return None,
    };
    let sql = match call_args.first()? {
        CallArg::StringLit(s) | CallArg::TemplateLit(s) => s.clone(),
        _ => return None,
    };
    let (entity, sql_op) = match parse_java_sql_entity(&sql) {
        Some(pair) => pair,
        None => return None,
    };
    let op = match (leaf_op, sql_op) {
        (DbQueryOp::Update, _) => DbQueryOp::Update,
        (DbQueryOp::Select, _) => DbQueryOp::Select,
        (DbQueryOp::Other, sql) => sql,
        _ => leaf_op,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("java.{}", entity),
        operation: op,
    })
}

fn is_jdbc_template_root(name: &str) -> bool {
    matches!(
        name,
        "jdbcTemplate"
            | "jdbc"
            | "namedJdbcTemplate"
            | "namedParameterJdbcTemplate"
            | "jdbcOperations"
    )
}

fn parse_java_sql_entity(
    sql: &str,
) -> Option<(String, crate::indexer::resolve::flow_emit::DbQueryOp)> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    let upper = sql.trim().to_ascii_uppercase();
    let (op, after) = if let Some(i) = upper.find("INSERT INTO ") {
        (DbQueryOp::Insert, i + 12)
    } else if let Some(i) = upper.find("UPDATE ") {
        (DbQueryOp::Update, i + 7)
    } else if let Some(i) = upper.find("DELETE FROM ") {
        (DbQueryOp::Delete, i + 12)
    } else if let Some(i) = upper.find(" FROM ") {
        (DbQueryOp::Select, i + 6)
    } else if upper.starts_with("FROM ") {
        (DbQueryOp::Select, 5)
    } else {
        return None;
    };
    let slice = sql.get(after..)?;
    let token = slice.split_whitespace().next()?;
    let entity: String = token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .to_string();
    if entity.is_empty() {
        return None;
    }
    Some((
        entity
            .rsplit('.')
            .next()
            .unwrap_or(entity.as_str())
            .to_string(),
        op,
    ))
}

// ---------------------------------------------------------------------------
// Retrofit `@GET("/x")` / `@POST(...)` etc. Producer HttpCall
// ---------------------------------------------------------------------------

pub(crate) fn detect_retrofit_attribute_emission(
    attr_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    let method = match attr_name {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "PATCH" => HttpMethod::Patch,
        "DELETE" => HttpMethod::Delete,
        "HEAD" => HttpMethod::Head,
        "OPTIONS" => HttpMethod::Options,
        "HTTP" => HttpMethod::Any,
        _ => return None,
    };
    let url = first_arg?.trim();
    if url.is_empty() {
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

// ---------------------------------------------------------------------------
// gRPC-java Stub → RpcCall Producer
// ---------------------------------------------------------------------------

/// `<Service>Grpc.newBlockingStub(channel).method(req)` /
/// `newFutureStub` / `newStub`. Detection: chain root ends with "Grpc",
/// second segment is one of the stub constructors, leaf is the rpc method.
pub(crate) fn detect_java_grpc_stub_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !root.ends_with("Grpc") || root == "Grpc" {
        return None;
    }
    let ctor = segs[1].name.as_str();
    if !matches!(ctor, "newBlockingStub" | "newFutureStub" | "newStub") {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    if matches!(leaf, "newBlockingStub" | "newFutureStub" | "newStub") {
        return None;
    }
    let service = root.strip_suffix("Grpc").unwrap_or(root);
    let name = format!("{}.{}", service, leaf);
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name,
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

