// =============================================================================
// languages/go/hooks.rs — GoHooks impl of LanguageEngineHooks plus the
// concrete GoResolver (chain-aware, scope-chain walk, same-package, import,
// dot-import, qualified-name, package-qualified via import alias, bare-name
// fallback with Go visibility) and external-classifier / flow-detector /
// file-context helpers it dispatches.
//
// Go's package-qualified calls (`gin.Default()`) drop the qualifier in the
// extractor — only the field_identifier (`Default`) lands in target_name.
// Disambiguation here happens via the file's imports.
// =============================================================================

pub(crate) use super::flow_detectors::{
    detect_go_bgjob_emission, detect_go_db_query_emission, detect_go_gorilla_ws_consumer,
    detect_go_grpc_chain_emission, detect_go_http_chain_emission, detect_go_mailer_emission,
    detect_go_mq_emission, detect_go_redis_config_lookup, detect_go_uds_emission,
};
use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    intern_yield_type, ChainMiss, FileContext, ImportEntry, RefContext, Resolution, SymbolInfo,
    SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::type_checker::type_env::TypeEnvironment;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};

pub struct GoResolver;

impl GoResolver {
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


        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = walk_go_chain(chain_ref, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }

            // Package-qualified call: chain = ["pkg", "Func"].
            if chain_ref.segments.len() >= 2 {
                let alias = &chain_ref.segments[0].name;
                if let Some(res) = self.resolve_via_import_alias(
                    file_ctx, alias, target, edge_kind, lookup,
                ) {
                    return Some(res);
                }
            }
        }

        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_same_package",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            // Direct members of the current package — O(tens) not O(all).
            for sym in lookup.members_of(pkg) {
                if sym.name == *target
                    && self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_same_package_by_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            // Dot import: exported names directly visible.
            if import.is_wildcard {
                let last_seg = full_path.rsplit('/').next().unwrap_or(full_path.as_str());
                let candidate = format!("{last_seg}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if self.is_visible(file_ctx, ref_ctx, sym)
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "go_dot_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
                continue;
            }

            let pkg_alias = import
                .alias
                .as_deref()
                .unwrap_or_else(|| full_path.rsplit('/').next().unwrap_or(full_path.as_str()));

            let last_seg = full_path.rsplit('/').next().unwrap_or(full_path.as_str());
            let candidate = format!("{last_seg}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            if pkg_alias != last_seg {
                let candidate = format!("{pkg_alias}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if self.is_visible(file_ctx, ref_ctx, sym)
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "go_import_alias",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }

        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
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

        if vis == "private" {
            if &*target.file_path == file_ctx.file_path {
                return true;
            }
            let target_dir = predicates::parent_dir(&target.file_path);
            let source_dir = predicates::parent_dir(&file_ctx.file_path);
            return target_dir == source_dir;
        }

        true
    }

    fn resolve_via_import_alias(
        &self,
        file_ctx: &FileContext,
        alias: &str,
        target: &str,
        edge_kind: EdgeKind,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            let import_alias = import
                .alias
                .as_deref()
                .unwrap_or_else(|| full_path.rsplit('/').next().unwrap_or(full_path.as_str()));

            if import_alias != alias {
                continue;
            }

            let pkg_name = full_path.rsplit('/').next().unwrap_or(full_path.as_str());
            let candidate = format!("{pkg_name}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "go_chain_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }

            if alias != pkg_name {
                let candidate = format!("{alias}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "go_chain_import",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }

            break;
        }
        None
    }
}

fn extract_package_name(file: &ParsedFile) -> Option<String> {
    for sym in &file.symbols {
        if let Some(dot) = sym.qualified_name.find('.') {
            let pkg = &sym.qualified_name[..dot];
            if !pkg.is_empty() {
                return Some(pkg.to_string());
            }
        }
        if let Some(ref sp) = sym.scope_path {
            if !sp.is_empty() {
                return Some(sp.split('.').next().unwrap_or(sp.as_str()).to_string());
            }
        }
    }
    None
}

// Returns false when the path matches (or is a sub-package of) the project's
// own module path. Falls back to dot-in-first-segment when no GoMod manifest.
pub(crate) fn is_manifest_go_external(ctx: &ProjectContext, import_path: &str) -> bool {
    let module_path: Option<&str> = ctx
        .manifest(ManifestKind::GoMod)
        .and_then(|m| m.module_path.as_deref());

    if let Some(module_path) = module_path {
        if import_path == module_path {
            return false;
        }
        if import_path.starts_with(module_path)
            && import_path.len() > module_path.len()
            && import_path.as_bytes()[module_path.len()] == b'/'
        {
            return false;
        }
        return true;
    }
    let first_segment = import_path.split('/').next().unwrap_or(import_path);
    first_segment.contains('.')
}

pub(crate) fn detect_flow_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let Some(chain) = r.chain.as_ref() else { return Vec::new(); };
    if let Some(emission) = detect_go_http_chain_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_db_query_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_grpc_chain_emission(chain, file_ctx) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_mailer_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_bgjob_emission(chain) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_mq_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_redis_config_lookup(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_uds_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_gorilla_ws_consumer(chain) {
        return vec![emission];
    }
    Vec::new()
}

pub(crate) fn detect_flow_inner_with_lookup(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let direct = detect_flow_inner(file_ctx, ref_ctx);
    if !direct.is_empty() {
        return direct;
    }
    // Let-binding propagation: `client := userpb.NewUserServiceClient(conn)`
    // followed by `client.GetUser(...)`. The variable's recorded type
    // comes from the Go extractor's TypeRef on the Variable symbol.
    let r = &ref_ctx.extracted_ref;
    let Some(chain) = r.chain.as_ref() else { return Vec::new() };
    let Some(root_seg) = chain.segments.first() else { return Vec::new() };
    if !matches!(root_seg.kind, crate::types::SegmentKind::Identifier) {
        return Vec::new();
    }
    let var_qname = match ref_ctx.source_symbol.scope_path.as_deref() {
        Some(scope) => format!("{}.{}", scope, root_seg.name),
        None => root_seg.name.clone(),
    };
    let type_name = match lookup.field_type_str(&var_qname) {
        Some(t) => t.to_string(),
        None => return Vec::new(),
    };
    if !type_name.ends_with("Client") {
        return Vec::new();
    }
    let mut new_segments = vec![
        crate::types::ChainSegment {
            name: type_name,
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
    ];
    new_segments.extend(chain.segments.iter().skip(1).cloned());
    let rewritten = crate::types::MemberChain { segments: new_segments };
    if let Some(em) = detect_go_grpc_chain_emission(&rewritten, file_ctx) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    let file_namespace = extract_package_name(file);

    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        let full_path = match &r.module {
            Some(m) => m.clone(),
            None => r.target_name.clone(),
        };

        let last_segment = full_path.rsplit('/').next().unwrap_or(&full_path);

        // Blank import (`import _ "path"`) — side effects only.
        if r.target_name == "_" {
            continue;
        }

        // Dot import (`import . "path"`) — exported names enter scope directly.
        let is_dot_import = r.target_name == ".";

        let alias = if is_dot_import || r.target_name == last_segment {
            None
        } else {
            Some(r.target_name.clone())
        };

        imports.push(ImportEntry {
            imported_name: alias.clone().unwrap_or_else(|| last_segment.to_string()),
            module_path: Some(full_path),
            alias,
            is_wildcard: is_dot_import,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "go".to_string(),
        imports,
        file_namespace,
    }
}

/// Go chain walker.
pub(crate) fn walk_go_chain(
    chain_ref: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain_ref.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: root type. Go has no `this`/`self`; first segment is always Identifier.
    let mut initial_generic_args: Vec<String> = Vec::new();
    let root_type = match segments[0].kind {
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            if let Some(local_type) = lookup.local_type(name) {
                Some(local_type)
            } else {
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "struct" | "interface" | "enum" | "type_alias"
                    )
                });
                if is_type {
                    Some(name.clone())
                } else {
                    let mut found = None;
                    for scope in &ref_ctx.scope_chain {
                        let field_qname = format!("{scope}.{name}");
                        if let Some(type_name) = lookup.field_type_str(&field_qname) {
                            initial_generic_args = lookup
                                .field_type_arg_strs(&field_qname)
                                .unwrap_or_default()
                                .to_vec();
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

    let mut current_type = match root_type {
        Some(t) => t,
        None => {
            // Package-qualified shape `pkg.Symbol(...)` — root is the
            // package alias, not a known type or variable. Record a
            // chain miss so demand-driven expand can pull the file
            // containing `Symbol` via the bare-name fallback.
            if let Some(last) = segments.last() {
                lookup.record_chain_miss(ChainMiss {
                    current_type: segments[0].name.clone(),
                    target_name: last.name.clone(),
                });
            }
            return None;
        }
    };
    let mut env = TypeEnvironment::new();

    if !initial_generic_args.is_empty() {
        env.enter_generic_context(&current_type, &initial_generic_args, |name| {
            lookup.generic_params(name).map(|p| p.to_vec())
        });
    }

    // Phase 2: intermediate segments.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_str(&member_qname) {
            let new_args = lookup
                .field_type_arg_strs(&member_qname)
                .unwrap_or_default()
                .to_vec();
            let resolved_type = env.resolve(&next_type);
            env.push_scope();
            if !new_args.is_empty() {
                env.enter_generic_context(&resolved_type, &new_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }
            current_type = resolved_type;
            continue;
        }

        if !seg.type_args.is_empty() {
            env.enter_generic_context(&member_qname, &seg.type_args, |name| {
                lookup.generic_params(name).map(|p| p.to_vec())
            });
        }

        if let Some(raw_return) = lookup.return_type_str(&member_qname) {
            let resolved = env.resolve(&raw_return);
            env.push_scope();
            current_type = resolved;
            continue;
        }

        let mut found = false;
        for sym in lookup.members_of(&current_type) {
            if sym.name != seg.name {
                continue;
            }
            if let Some(ft) = lookup.field_type_str(&sym.qualified_name) {
                let resolved_type = env.resolve(&ft);
                env.push_scope();
                current_type = resolved_type;
                found = true;
                break;
            }
            if let Some(rt) = lookup.return_type_str(&sym.qualified_name) {
                let resolved = env.resolve(&rt);
                env.push_scope();
                current_type = resolved;
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
                strategy = "go_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "go_chain_resolution",
                resolved_yield_type: intern_yield_type(generic_yield_type(sym, &last.type_args, lookup, &mut env), lookup),
                flow_emit: None,
            });
        }
    }

    for sym in lookup.members_of(&current_type) {
        if sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "go_chain_resolution",
                resolved_yield_type: intern_yield_type(generic_yield_type(sym, &last.type_args, lookup, &mut env), lookup),
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

/// Yield type honoring call-site generics.
fn generic_yield_type(
    sym: &SymbolInfo,
    call_site_type_args: &[String],
    lookup: &dyn SymbolLookup,
    env: &mut TypeEnvironment,
) -> Option<String> {
    let raw = lookup
        .return_type_str(&sym.qualified_name)
        .or_else(|| lookup.field_type_str(&sym.qualified_name))?;
    if !call_site_type_args.is_empty() {
        env.enter_generic_context(&sym.qualified_name, call_site_type_args, |name| {
            lookup.generic_params(name).map(|p| p.to_vec())
        });
    }
    Some(env.resolve(&raw))
}

pub struct GoHooks;

impl LanguageEngineHooks for GoHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            return Some(import_path.to_string());
        }

        if predicates::is_go_builtin(target) || predicates::is_go_composite_type(target) {
            return Some("builtin".to_string());
        }

        let is_exported = target.chars().next().is_some_and(|c| c.is_uppercase());
        if !is_exported {
            return None;
        }

        // Prefer the longest matching external module path. Manifest-driven
        // via go.mod when ProjectContext is available.
        let mut best: Option<&str> = None;
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };
            let external = if let Some(ctx) = project_ctx {
                is_manifest_go_external(ctx, full_path)
            } else {
                predicates::is_external_go_import_fallback(full_path)
            };
            if external && (best.is_none() || full_path.len() > best.unwrap().len()) {
                best = Some(full_path.as_str());
            }
        }
        best.map(|s| s.to_string())
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
        if let Some(res) = GoResolver.resolve(file_ctx, ref_ctx, lookup) {
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

pub static GO_HOOKS: GoHooks = GoHooks;
