// HCL / Terraform language hooks. Absorbed from the deleted `hcl/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct HclHooks;

pub(crate) fn is_terraform_meta_ref(name: &str) -> bool {
    matches!(
        name.splitn(2, '.').next().unwrap_or(name),
        "each" | "count" | "self" | "path" | "terraform"
    )
}

pub(crate) fn is_dynamic_block_iterator(name: &str) -> bool {
    let mut parts = name.split('.');
    let head = match parts.next() {
        Some(h) if !h.is_empty() => h,
        _ => return false,
    };
    let tail = match parts.next() {
        Some(t) => t,
        None => return false,
    };
    if !matches!(tail, "value" | "key") {
        return false;
    }
    head.chars()
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false)
        && head.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn is_provider_resource_type(name: &str) -> bool {
    name.contains('_')
        && (name.starts_with("aws_")
            || name.starts_with("azurerm_")
            || name.starts_with("google_")
            || name.starts_with("kubernetes_")
            || name.starts_with("helm_")
            || name.starts_with("null_")
            || name.starts_with("random_")
            || name.starts_with("local_")
            || name.starts_with("tls_")
            || name.starts_with("vault_")
            || name.starts_with("consul_")
            || name.starts_with("nomad_"))
}

fn strip_hcl_prefix(name: &str) -> &str {
    for prefix in ["var.", "local."] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return rest.splitn(2, '.').next().unwrap_or(rest);
        }
    }
    name
}

impl LanguageEngineHooks for HclHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some("terraform".to_string());
        }
        if is_terraform_meta_ref(target) {
            return Some("terraform".to_string());
        }
        if is_dynamic_block_iterator(target) {
            return Some("terraform".to_string());
        }
        if target.starts_with("data.") {
            let parts: Vec<&str> = target.splitn(3, '.').collect();
            if parts.len() >= 2 && is_provider_resource_type(parts[1]) {
                return Some("terraform".to_string());
            }
        }
        if let Some(dot) = target.find('.') {
            let prefix = &target[..dot];
            if is_provider_resource_type(prefix) {
                return Some("terraform".to_string());
            }
        }
        None
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "hcl".to_string(),
            imports: Vec::new(),
            file_namespace: None,
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
        if is_terraform_meta_ref(target) {
            return None;
        }
        if let Some(sym) = lookup.by_qualified_name(target) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "hcl_qname_direct",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
        if let Some(dot_pos) = target.find('.') {
            let head = &target[..dot_pos];
            if !head.contains('_') && !head.is_empty() {
                for sym in lookup.in_file(&file_ctx.file_path) {
                    if sym.name == head && sym.kind == "class" {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "hcl_provider_alias",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
                for sym in lookup.by_name(head) {
                    if sym.kind == "class" {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.85,
                            strategy: "hcl_provider_alias_cross_file",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }
        let bare = strip_hcl_prefix(target);
        if bare != target.as_str() {
            for sym in lookup.in_file(&file_ctx.file_path) {
                if sym.name == bare {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "hcl_same_file_bare",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
            for sym in lookup.by_name(bare) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.9,
                    strategy: "hcl_cross_file_bare",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: |_, _| true,
        })
        .resolve_all()
    }
}

pub static HCL_HOOKS: HclHooks = HclHooks;
