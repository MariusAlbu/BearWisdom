// =============================================================================
// indexer/resolve/rules/scala/mod.rs — Scala resolution rules
//
// Scope rules for Scala:
//
//   1. Scope chain walk: innermost class/object/function → outermost.
//   2. Same-package resolution: types in the same package are visible without
//      explicit import (Scala package visibility).
//   3. Exact import resolution: `import com.foo.Bar` → Bar directly visible.
//   4. Wildcard import: `import com.foo._` → all types in that package visible.
//   5. Fully qualified names: dotted names resolve directly.
//
// Scala import model:
//   `import com.foo.Bar`   → target_name = "Bar",  module = "com.foo.Bar"
//   `import com.foo._`     → target_name = "_",    module = "com.foo"
//   `import com.foo.{A, B}` → two separate Imports refs
// =============================================================================


use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::type_checker::chain::{
    self, ChainConfig, NamespaceLookup, identity_normalize,
};
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Scala language resolver.
pub struct ScalaResolver;

impl LanguageResolver for ScalaResolver {
    fn language_ids(&self) -> &[&str] {
        &["scala"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract package declaration.
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
            // Scala wildcard is `_` (Scala 2) or `*` (Scala 3).
            let is_wildcard = r.target_name == "_" || r.target_name == "*";

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
            language: "scala".to_string(),
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

        // Global external lookup for bare names. Prefers chain walker context
        // when the ref has a real multi-segment chain, but falls back here
        // when the chain is a single-segment stub (common for ScalaTest DSL
        // calls like `equal` or `be` extracted from infix expressions where
        // the receiver is a separate ref) or when chain resolution fails.
        let try_external_global = |target: &str| -> Option<Resolution> {
            if target.contains('.') { return None; }
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") { continue; }
                if !predicates::kind_compatible(edge_kind, &sym.kind) { continue; }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "scala_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
            None
        };

        // Single-segment chain or no chain: try external global first.
        let chain_is_single_segment = ref_ctx.extracted_ref.chain
            .as_ref()
            .map(|c| c.segments.len() <= 1)
            .unwrap_or(false);
        if ref_ctx.extracted_ref.chain.is_none() || chain_is_single_segment {
            if let Some(res) = try_external_global(target) {
                return Some(res);
            }
        }

        // Chain-aware resolution: walk MemberChain following field/return types.
        // Scala is JVM-like with wildcard imports and companion objects.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            let config = ChainConfig {
                strategy_prefix: "scala",
                normalize_type: identity_normalize,
                has_self_ref: true,
                enclosing_type_kinds: &["class", "trait", "object"],
                static_type_kinds: &["class", "trait", "object", "enum", "type_alias"],
                use_generics: true,
                namespace_lookup: NamespaceLookup::WildcardOnly,
                kind_compatible: predicates::kind_compatible,
            };
            if let Some(res) = chain::resolve_via_chain(
                &config, chain_val, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
                return Some(res);
            }
            // Chain resolution failed — fall back to external global for
            // DSL methods whose receiver type is unavailable (e.g. ScalaTest
            // matchers on the right-hand side of an infix chain).
            if let Some(res) = try_external_global(target) {
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
                        strategy: "scala_scope_chain",
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
                        strategy: "scala_same_package",
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
                            strategy: "scala_import",
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
                            strategy: "scala_wildcard_import",
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
                        strategy: "scala_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Step 6: Implicit imports — Scala compiles every file with the
        // equivalent of `import java.lang._; import scala._; import
        // scala.Predef._`. Bare references to `Throwable`, `Exception`,
        // `Object`, `Class`, `Number`, `Integer`, etc. are valid Scala
        // because java.lang is implicitly imported. Try the three implicit
        // namespaces in compiler order; first-hit wins.
        if !effective_target.contains('.') {
            for prefix in &["java.lang", "scala", "scala.Predef"] {
                let candidate = format!("{prefix}.{effective_target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "scala_implicit_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
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
            "public" => true,
            "protected" => true,
            "private" => &*target.file_path == file_ctx.file_path,
            // `private[pkg]` / `protected[pkg]` — allow (full check needs package graph).
            _ => true,
        }
    }

    fn detect_flow_emission(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let r = &ref_ctx.extracted_ref;
        if r.kind != EdgeKind::Calls {
            return Vec::new();
        }
        if let Some(chain) = r.chain.as_ref() {
            if let Some(em) = detect_scala_http_chain_emission(chain, &r.call_args) {
                return vec![em];
            }
            if let Some(em) = detect_scala_db_query_emission(chain, &r.call_args) {
                return vec![em];
            }
            if let Some(em) = detect_scala_doobie_emission(chain) {
                return vec![em];
            }
            if let Some(em) = detect_scala_quill_emission(chain) {
                return vec![em];
            }
            if let Some(em) = detect_scala_zio_sql_emission(chain, &r.call_args) {
                return vec![em];
            }
            if let Some(em) = detect_scala_grpc_emission(chain) {
                return vec![em];
            }
        }
        // http4s / Akka HTTP `path("x") { get { ... } }` — bare calls.
        if let Some(em) = detect_scala_http_path_call(r.target_name.as_str(), &r.call_args) {
            return vec![em];
        }
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Scala HTTP detection
// ---------------------------------------------------------------------------

/// sttp `basicRequest.get(uri"...")` / Akka HTTP `Http().singleRequest(...)`.
/// Also covers WS Producer for `client.get("/x")`-style chains.
pub(crate) fn detect_scala_http_chain_emission(
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

/// http4s / Akka HTTP `path("users") { ... }`, route DSL — Consumer.
pub(crate) fn detect_scala_http_path_call(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if !matches!(target_name, "path" | "pathPrefix" | "pathEnd") {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.as_str(),
        _ => return None,
    };
    let normalised = if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{}", url)
    };
    let name = crate::connectors::url_pattern::normalize(&normalised);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(HttpMethod::Any),
    streaming: None,
    })
}

/// Slick `users.filter(_.id === id).result` — chain ends in `.result`,
/// `.first`, etc. on a TableQuery. Doobie `sql"SELECT ...".query[X]`. For
/// minimal v1 we match chain-root PascalCase + Slick op leaf.
pub(crate) fn detect_scala_db_query_emission(
    chain: &crate::types::MemberChain,
    _call_args: &[crate::types::CallArg],
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
        "result" | "list" | "headOption" | "to" | "filter" | "map" | "exists" => DbQueryOp::Select,
        "insert" | "+=" | "++=" | "insertOrUpdate" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("scala.{}", root),
        operation: op,
    })
}

/// Doobie `sql"SELECT ... FROM users".query[User].option`.
/// Chain has a leading `sql` segment (the interpolator). Leaf op tells us
/// the result-set verb. Entity is parsed from a StringInterpolation arg's
/// FROM/INTO/UPDATE/DELETE token when available; falls back to "*".
pub(crate) fn detect_scala_doobie_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let has_sql_seg = segs.iter().any(|s| s.name == "sql");
    if !has_sql_seg {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let op = match leaf {
        "query" | "option" | "unique" | "nel" | "to" | "stream" | "list" => DbQueryOp::Select,
        "update" | "run" => DbQueryOp::Update,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "scala.doobie".to_string(),
        operation: op,
    })
}

/// Quill `quote { query[User].filter(_.id == lift(id)) }`. The chain root
/// or an inner segment is `query[T]` with an entity type parameter. Match
/// on the segment name `query` and treat the first declared type-arg as
/// the entity hint.
pub(crate) fn detect_scala_quill_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let segs = &chain.segments;
    let query_seg = segs.iter().find(|s| s.name == "query")?;
    let entity = query_seg
        .type_args
        .first()
        .cloned()
        .unwrap_or_else(|| "*".to_string());
    let leaf = segs.last()?.name.as_str();
    let op = match leaf {
        "filter" | "map" | "sortBy" | "size" | "take" | "drop" | "groupBy" => DbQueryOp::Select,
        "insert" | "insertValue" => DbQueryOp::Insert,
        "update" | "updateValue" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("scala.{}", entity),
        operation: op,
    })
}

/// ZIO SQL `select(*).from(users).where(...)`. The shape `.from(<Ident>)`
/// names the table directly. Match on the leaf `from` and grab the first
/// Ident argument as the entity name.
pub(crate) fn detect_scala_zio_sql_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    let leaf = chain.segments.last()?.name.as_str();
    if leaf != "from" {
        return None;
    }
    // The chain must contain a `select` segment earlier — otherwise
    // we'd match arbitrary `foo.from(bar)` calls.
    if !chain.segments.iter().any(|s| s.name == "select") {
        return None;
    }
    let entity = call_args.iter().find_map(|a| match a {
        CallArg::Ident(name) if !name.is_empty() => Some(name.clone()),
        _ => None,
    })?;
    Some(FlowEmission::DbQuery {
        entity_name: format!("scala.{}", entity),
        operation: DbQueryOp::Select,
    })
}

pub(crate) fn detect_scala_grpc_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !root.ends_with("Grpc") && !root.ends_with("Client") {
        return None;
    }
    let ctor = segs[1].name.as_str();
    if !matches!(ctor, "stub" | "blockingStub" | "newStub" | "newBlockingStub" | "apply") {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    if matches!(leaf, "stub" | "blockingStub" | "newStub" | "newBlockingStub" | "apply") {
        return None;
    }
    let service = root
        .strip_suffix("Grpc")
        .or_else(|| root.strip_suffix("Client"))
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

    // Import refs.
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

        if predicates::is_external_scala_namespace(import_path, project_ctx) {
            return Some(import_path.to_string());
        }
        return None;
    }

    // Walk imports for a match.
    for import in &file_ctx.imports {
        let ns = import.module_path.as_deref().unwrap_or("");
        if ns.is_empty() {
            continue;
        }
        if !import.is_wildcard
            && import.imported_name != *target
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

        if predicates::is_external_scala_namespace(ns, project_ctx) {
            return Some(ns.to_string());
        }
    }

    // Fully-qualified target.
    if predicates::effective_target_is_external(target, project_ctx) {
        return Some(target.clone());
    }

    None
}
