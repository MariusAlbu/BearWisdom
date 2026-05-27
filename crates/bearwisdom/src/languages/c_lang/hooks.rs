// C/C++ language hooks. Absorbed from the deleted `c_lang/resolve.rs`.

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, intern_yield_type, ChainMiss, FileContext, ImportEntry, RefContext, Resolution,
    SymbolLookup,
};
use crate::type_checker::chain::simple_yield_type;
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};
use tracing::debug;

pub struct CHooks;

pub(crate) const R_PACKAGE_SENTINEL: &str = "__r_package__";

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let target = r.target_name.as_str();
    if matches!(
        target,
        "PQexec" | "PQexecParams" | "sqlite3_prepare_v2" | "sqlite3_exec" | "mysql_query"
    ) {
        let sql = r.call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s) => Some(s.as_str()),
            _ => None,
        });
        if let Some(sql) = sql {
            let upper = sql.to_ascii_uppercase();
            let op = if upper.contains("INSERT INTO") {
                DbQueryOp::Insert
            } else if upper.contains("UPDATE ") {
                DbQueryOp::Update
            } else if upper.contains("DELETE FROM") {
                DbQueryOp::Delete
            } else if upper.contains(" FROM ") || upper.starts_with("SELECT") {
                DbQueryOp::Select
            } else {
                DbQueryOp::Other
            };
            return vec![FlowEmission::DbQuery {
                entity_name: "c.*".to_string(),
                operation: op,
            }];
        }
    }
    Vec::new()
}

impl LanguageEngineHooks for CHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if file_ctx.file_namespace.as_deref() == Some(R_PACKAGE_SENTINEL)
            && predicates::is_r_c_api_symbol(target)
        {
            return Some("r.c.api".to_string());
        }
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let header = target.trim_matches(|c| c == '<' || c == '>' || c == '"');
            if predicates::is_system_header(header) {
                return Some("stdlib".to_string());
            }
            if header.starts_with("boost/")
                || header.starts_with("gtest/")
                || header.starts_with("gmock/")
            {
                return Some("external".to_string());
            }
            return None;
        }
        if predicates::is_template_param(target) {
            return Some("template_param".to_string());
        }
        if target.starts_with("std::") || target.starts_with("::std::") {
            return Some("std".to_string());
        }
        let root = target
            .strip_prefix("::")
            .unwrap_or(target)
            .split("::")
            .next()
            .unwrap_or(target);
        if predicates::is_external_c_namespace(root) {
            return Some(root.to_string());
        }
        None
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        _lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let header = r.target_name.trim_matches(|c| c == '<' || c == '>' || c == '"');
            imports.push(ImportEntry {
                imported_name: header.to_string(),
                module_path: Some(header.to_string()),
                alias: None,
                is_wildcard: false,
            });
        }
        let file_namespace = if project_ctx
            .map(|ctx| ctx.manifests.contains_key(&ManifestKind::Description))
            .unwrap_or(false)
        {
            Some(R_PACKAGE_SENTINEL.to_string())
        } else {
            None
        };
        Some(FileContext {
            file_path: file.path.clone(),
            language: file.language.clone(),
            imports,
            file_namespace,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if predicates::is_template_param(target) {
            return None;
        }
        if let Some(chain_ref) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = walk_c_lang_chain(chain_ref, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }
        if file_ctx.file_namespace.as_deref() == Some(R_PACKAGE_SENTINEL)
            && predicates::is_r_c_api_symbol(target)
        {
            return None;
        }
        let effective_target = target
            .strip_prefix("this->")
            .or_else(|| target.strip_prefix("this."))
            .unwrap_or(target);
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}::{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "c_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        if effective_target.contains("::") {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "c_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "c_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        // External stdlib globals (printf, malloc, …). Runs after scope,
        // qualified, and same-file strategies so a project symbol wins over a
        // same-named external.
        if ref_ctx.extracted_ref.chain.is_none()
            && !target.contains("::")
            && !target.contains('.')
            && !target.contains("->")
        {
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
                    strategy: "c_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        if !effective_target.contains("::") && effective_target.len() > 1 {
            let by_name_hits = lookup.by_name(effective_target);
            if let Some(sym) = by_name_hits.first() {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "c_by_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        if let Some(res) = (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all() {
            return Some(res);
        }
        if matches!(
            edge_kind,
            EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates
        ) && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
            && !target.contains("::")
        {
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
}

/// C/C++ chain walker.
pub(crate) fn walk_c_lang_chain(
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
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            if let Some(local_type) = lookup.local_type(name) {
                Some(local_type)
            } else {
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "class" | "struct" | "interface" | "enum" | "type_alias"
                    )
                });
                if is_type {
                    Some(name.clone())
                } else {
                    let mut found = None;
                    for scope in &ref_ctx.scope_chain {
                        let field_qname = format!("{scope}.{name}");
                        if let Some(type_name) = lookup.field_type_str(&field_qname) {
                            found = Some(type_name.to_string());
                            break;
                        }
                        let field_qname_cc = format!("{}.{name}", scope.replace("::", "."));
                        if let Some(type_name) = lookup.field_type_str(&field_qname_cc) {
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
    current_type = normalize_type(&current_type);

    // Phase 2: intermediate segments.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_str(&member_qname) {
            current_type = normalize_type(&next_type);
            current_type = dereference_typedef(&current_type, lookup);
            continue;
        }

        if let Some(raw_return) = lookup.return_type_str(&member_qname) {
            current_type = normalize_type(&raw_return);
            current_type = dereference_typedef(&current_type, lookup);
            continue;
        }

        let mut found = false;
        for sym in lookup.members_of(&current_type) {
            if sym.name != seg.name {
                continue;
            }
            if let Some(ft) = lookup.field_type_str(&sym.qualified_name) {
                current_type = normalize_type(&ft);
                current_type = dereference_typedef(&current_type, lookup);
                found = true;
                break;
            }
            if let Some(rt) = lookup.return_type_str(&sym.qualified_name) {
                current_type = normalize_type(&rt);
                current_type = dereference_typedef(&current_type, lookup);
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
    current_type = dereference_typedef(&current_type, lookup);
    let candidate = format!("{current_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if predicates::kind_compatible(edge_kind, &sym.kind) {
            debug!(
                strategy = "c_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "c_chain_resolution",
                resolved_yield_type: intern_yield_type(
                    simple_yield_type(sym, lookup).map(|t| normalize_type(&t)),
                    lookup,
                ),
                flow_emit: None,
            });
        }
    }

    let matches: Vec<_> = lookup
        .members_of(&current_type)
        .iter()
        .filter(|sym| sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind))
        .cloned()
        .collect();

    match matches.len() {
        0 => {
            lookup.record_chain_miss(ChainMiss {
                current_type: current_type.clone(),
                target_name: last.name.clone(),
            });
            None
        }
        1 => Some(Resolution {
            target_symbol_id: matches[0].id,
            confidence: 1.0,
            strategy: "c_chain_resolution_unique",
            resolved_yield_type: intern_yield_type(
                simple_yield_type(&matches[0], lookup).map(|t| normalize_type(&t)),
                lookup,
            ),
            flow_emit: None,
        }),
        _ => Some(Resolution {
            target_symbol_id: matches[0].id,
            confidence: 0.95,
            strategy: "c_chain_resolution",
            resolved_yield_type: intern_yield_type(
                simple_yield_type(&matches[0], lookup).map(|t| normalize_type(&t)),
                lookup,
            ),
            flow_emit: None,
        }),
    }
}

/// Find the enclosing class name from the scope chain.
fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct") {
                return Some(scope.clone());
            }
        }
        let normalized = scope.replace("::", ".");
        if normalized != *scope {
            if let Some(sym) = lookup.by_qualified_name(&normalized) {
                if matches!(sym.kind.as_str(), "class" | "struct") {
                    return Some(normalized);
                }
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(normalize_type(&scope_chain[scope_chain.len() - 2]));
    }
    scope_chain.last().map(|s| normalize_type(s))
}

/// Normalize a C++ qualified name for type_info lookups (`::` → `.`).
fn normalize_type(name: &str) -> String {
    name.replace("::", ".")
}

/// One-hop typedef dereference: project-defined pointer typedefs like
/// `TSocketChannelPtr` → `SocketChannel`.
fn dereference_typedef(type_name: &str, lookup: &dyn SymbolLookup) -> String {
    for sym in lookup.types_by_name(type_name) {
        if sym.kind == "type_alias" {
            if let Some(aliased) = lookup.field_type_str(&sym.qualified_name) {
                return normalize_type(&aliased);
            }
        }
    }
    type_name.to_string()
}

pub static C_HOOKS: CHooks = CHooks;
