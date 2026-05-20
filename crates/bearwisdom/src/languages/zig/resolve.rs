// =============================================================================
// zig/resolve.rs — Zig resolution rules
//
// Scope rules for Zig:
//
//   1. Scope chain walk: innermost fn/struct → outermost.
//   2. Same-file resolution: all top-level declarations visible within the file.
//   3. Import-based resolution:
//        `const mod = @import("module.zig")` → brings `mod` into scope
//        `const std = @import("std")`        → standard library (external)
//
// The extractor emits EdgeKind::Imports with:
//   target_name = the local binding name (e.g., "std", "mod")
//   module      = the @import argument string (e.g., "std", "module.zig")
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Zig language resolver.
pub struct ZigResolver;

impl LanguageResolver for ZigResolver {
    fn language_ids(&self) -> &[&str] {
        &["zig"]
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
            // target_name = local alias (const name), module = @import argument
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            let alias = if r.module.is_some() {
                Some(r.target_name.clone())
            } else {
                None
            };

            imports.push(ImportEntry {
                imported_name: module_path.clone(),
                module_path: Some(module_path),
                alias,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "zig".to_string(),
            imports,
            file_namespace: None,
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

        if predicates::is_zig_builtin(target) {
            return None;
        }

        engine::resolve_common("zig", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn detect_flow_emission(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        use crate::indexer::resolve::flow_emit::{
            ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
        };
        use crate::types::CallArg;
        let r = &ref_ctx.extracted_ref;
        if r.kind != EdgeKind::Calls {
            return Vec::new();
        }
        // std.http.Client send / fetch — Producer.
        let target = r.target_name.as_str();
        if matches!(target, "fetch" | "send" | "open") {
            let url = r.call_args.iter().find_map(|a| match a {
                CallArg::StringLit(s)
                    if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
                {
                    Some(s.as_str())
                }
                _ => None,
            });
            if let Some(url) = url {
                return vec![FlowEmission::NamedChannel {
                    kind: NamedChannelKind::HttpCall,
                    name: crate::connectors::url_pattern::normalize(url),
                    role: ChannelRole::Producer,
                    method: Some(HttpMethod::Any),
                streaming: None,
                }];
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
