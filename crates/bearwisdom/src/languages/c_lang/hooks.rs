// C/C++ language hooks. Absorbed from the deleted `c_lang/resolve.rs`.

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct CHooks;

/// C/C++ `ChainConfig` for the unified `resolve_via_chain`.
///
/// C carries one delta the other base languages don't: `normalize_type` rewrites
/// C++ `::` qualifiers to the `.` separator the index keys on. `expand_aliases`
/// collapses project-defined pointer typedefs (`TSocketChannelPtr` → `SocketChannel`)
/// through the generic alias-expansion path — no separate fn-pointer needed.
/// `walk_inheritance` is on so the members-of final fallback fires. No generics,
/// no namespace lookups, no external-qname promotion, no construction roots.
pub(crate) static C_LANG_CHAIN_CONFIG: crate::type_checker::chain::ChainConfig =
    crate::type_checker::chain::ChainConfig {
        strategy_prefix: "c",
        normalize_type: normalize_type,
        has_self_ref: true,
        enclosing_type_kinds: &["class", "struct"],
        static_type_kinds: &["class", "struct", "interface", "enum", "type_alias"],
        use_generics: false,
        namespace_lookup: crate::type_checker::chain::NamespaceLookup::None,
        kind_compatible: predicates::kind_compatible,
        extensions: crate::type_checker::chain::ChainExtensions {
            expand_aliases: true,
            walk_inheritance: true,
            promote_external_qname: false,
            root_construction: false,
            extension_method_fallback: false,
            root_fallback: None,
            root_type_access: false,
            qualify_via_imports: false,
        },
    };

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
            if let Some(res) = crate::type_checker::chain::resolve_via_chain(
                &C_LANG_CHAIN_CONFIG, chain_ref, edge_kind, Some(file_ctx), ref_ctx, lookup,
            ) {
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
                        module: None,
                    },
                );
            }
        }
        None
    }
}

/// Normalize a C++ qualified name for type_info lookups (`::` → `.`).
fn normalize_type(name: &str) -> String {
    name.replace("::", ".")
}

pub static C_HOOKS: CHooks = CHooks;
