// Protocol Buffers hooks. Absorbed from the deleted `proto/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, SymbolLookup,
};
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

/// Proto targets the generic resolver must not bind to a project symbol: the
/// scalar types and the `google.protobuf.*` well-known types. Tolerates the
/// leading-dot form (`.google.protobuf.Timestamp`) the extractor emits for
/// fully-qualified references.
pub(crate) fn is_proto_builtin(name: &str) -> bool {
    let bare = name.trim_start_matches('.');
    is_proto_scalar(bare) || bare.starts_with("google.protobuf.")
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
}

pub static PROTO_HOOKS: ProtoHooks = ProtoHooks;
