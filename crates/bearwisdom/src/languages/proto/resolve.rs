// =============================================================================
// languages/proto/resolve.rs — Protocol Buffers resolution rules
//
// Protobuf references:
//
//   import "google/protobuf/timestamp.proto";   → Imports
//   import "other_service.proto";               → Imports
//   message Request { UserInfo user = 1; }      → TypeRef, target_name = "UserInfo"
//   rpc GetUser(GetUserRequest) returns (User); → TypeRef, target_name = "User"
//   stream MyMessage                            → TypeRef
//
// Resolution strategy:
//   1. Same-file: all messages/enums defined in the same .proto file.
//   2. Import-based: for each imported proto, search by name within its file.
//   3. Package-qualified lookup: `{package}.{name}`.
//   4. Global name fallback.
//
// External namespace:
//   - `"protobuf"` for well-known types (google.protobuf.*)
//   - `"protobuf"` for scalar types
// =============================================================================

use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct ProtoResolver;

impl LanguageResolver for ProtoResolver {
    fn language_ids(&self) -> &[&str] {
        &["proto", "protobuf"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // Import path like "google/protobuf/timestamp.proto" or "other.proto".
            let path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(path),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "proto".to_string(),
            imports,
            file_namespace: file
                .symbols
                .iter()
                .find(|s| s.kind.as_str() == "package" || s.name.starts_with("package"))
                .map(|s| s.name.clone()),
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

        // Import declarations are file-level, not symbol refs.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Only TypeRef edges carry type name references in proto.
        if edge_kind != EdgeKind::TypeRef {
            return None;
        }

        // Proto scalar types are external.
        if is_proto_scalar(target) {
            return None;
        }

        // Well-known google.protobuf.* types are external.
        if target.starts_with("google.protobuf.") {
            return None;
        }

        // Step 1: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "proto_same_file",
                });
            }
        }

        // Step 2: Package-qualified lookup.
        // Fully qualified names like ".mypackage.MyMessage" (leading dot).
        let bare_target = target.trim_start_matches('.');
        if let Some(sym) = lookup.by_qualified_name(bare_target) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "proto_qualified",
            });
        }

        // Try with the file's own package prefix.
        if let Some(pkg) = &file_ctx.file_namespace {
            let candidate = format!("{pkg}.{bare_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "proto_package_qualified",
                });
            }
        }

        // Step 3: Global name lookup (from imported .proto files).
        for sym in lookup.by_name(bare_target) {
            if matches!(sym.kind.as_str(), "struct" | "enum" | "class") {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.9,
                    strategy: "proto_global_type",
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if is_proto_scalar(target) {
            return Some("protobuf".to_string());
        }

        if target.starts_with("google.protobuf.") || target.starts_with(".google.protobuf.") {
            return Some("protobuf".to_string());
        }

        // google/protobuf/* imports are well-known types.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let module = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            if module.starts_with("google/protobuf/") {
                return Some("protobuf".to_string());
            }
        }

        None
    }
}

/// Protobuf scalar (primitive) types.
fn is_proto_scalar(name: &str) -> bool {
    matches!(
        name,
        "double" | "float" | "int32" | "int64" | "uint32" | "uint64"
            | "sint32" | "sint64" | "fixed32" | "fixed64"
            | "sfixed32" | "sfixed64" | "bool" | "string" | "bytes"
    )
}
