use super::resolve;
use super::predicates;
use super::resolve::is_manifest_external_namespace;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct FsharpHooks;

impl LanguageEngineHooks for FsharpHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs (`open System.Linq`) — classify via NuGet manifest.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, target),
                None => predicates::is_external_namespace_fallback(target),
            };
            if external {
                let root = target.split('.').next().unwrap_or(target);
                return Some(root.to_string());
            }
            return None;
        }

        // Module-qualified ref.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, module),
                None => predicates::is_external_namespace_fallback(module),
            };
            if external {
                let root = module.split('.').next().unwrap_or(module);
                return Some(root.to_string());
            }
        }

        // File's open declarations.
        for import in &file_ctx.imports {
            let Some(module_path) = &import.module_path else { continue };
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, module_path),
                None => predicates::is_external_namespace_fallback(module_path),
            };
            if external {
                let root = module_path.split('.').next().unwrap_or(module_path);
                return Some(root.to_string());
            }
        }

        None
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        resolve::detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }
}

pub static FSHARP_HOOKS: FsharpHooks = FsharpHooks;
