// =============================================================================
// languages/csharp/hooks.rs — CSharpHooks impl of LanguageEngineHooks.
//
// First migration: `classify_external` hosts the body of the legacy
// `CSharpResolver::infer_external_namespace`. Helpers
// `matches_workspace_project` and `is_manifest_external_namespace` are
// reused from `resolve` at `pub(super)` visibility.
// =============================================================================

use super::resolve;
use super::predicates;
use super::resolve::{is_manifest_external_namespace, matches_workspace_project};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct CSharpHooks;

impl LanguageEngineHooks for CSharpHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs (e.g., `using System.Linq;`) — classify the using
        // directive itself as external if the namespace is known-external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            if let Some(ctx) = project_ctx {
                if matches_workspace_project(ctx, target) {
                    return None;
                }
            }
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, target),
                None => predicates::is_external_namespace_fallback(target),
            };
            if external {
                return Some(target.clone());
            }
            return None;
        }

        // Check file's using directives (includes global usings from
        // ProjectContext) for external namespaces. Return the most
        // specific (longest) match.
        let mut best: Option<&str> = None;
        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }

            if let Some(ctx) = project_ctx {
                if matches_workspace_project(ctx, ns) {
                    continue;
                }
            }

            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, ns),
                None => predicates::is_external_namespace_fallback(ns),
            };

            if external && (best.is_none() || ns.len() > best.unwrap().len()) {
                best = Some(ns);
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

pub static CSHARP_HOOKS: CSharpHooks = CSharpHooks;
