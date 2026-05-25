// Protocol Buffers hooks. Absorbed from the deleted `proto/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct ProtoHooks;

/// Protobuf scalar (primitive) types.
pub(crate) fn is_proto_scalar(name: &str) -> bool {
    matches!(
        name,
        "double"
            | "float"
            | "int32"
            | "int64"
            | "uint32"
            | "uint64"
            | "sint32"
            | "sint64"
            | "fixed32"
            | "fixed64"
            | "sfixed32"
            | "sfixed64"
            | "bool"
            | "string"
            | "bytes"
    )
}

impl LanguageEngineHooks for ProtoHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, is_proto_scalar)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(path),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "proto".to_string(),
            imports,
            file_namespace: file
                .symbols
                .iter()
                .find(|s| s.kind.as_str() == "package" || s.name.starts_with("package"))
                .map(|s| s.name.clone()),
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
        if edge_kind != EdgeKind::TypeRef {
            return None;
        }
        if is_proto_scalar(target) {
            return None;
        }
        if target.starts_with("google.protobuf.") {
            return None;
        }
        let bare_target = target.trim_start_matches('.');
        if let Some(sym) = lookup.by_qualified_name(bare_target) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "proto_qualified",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{bare_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "proto_package_qualified",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: |_, sym_kind| {
                matches!(sym_kind, "struct" | "enum" | "class")
            },
        })
        .resolve_all()
    }
}

pub static PROTO_HOOKS: ProtoHooks = ProtoHooks;
