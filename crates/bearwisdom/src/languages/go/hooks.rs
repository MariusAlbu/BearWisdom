// =============================================================================
// languages/go/hooks.rs — GoHooks impl of LanguageEngineHooks.
//
// Hosts engine-side per-language behaviors that used to live on
// `GoResolver` (the LanguageResolver impl). First migration:
// `classify_external` ports the body of
// `GoResolver::infer_external_namespace` onto the engine hooks seam.
// The legacy method stays in place during the migration — engine hook
// fires FIRST in loop_body.rs Tier 1.5; legacy runs as fallback. When
// every language has migrated, the legacy method comes off the
// LanguageResolver trait entirely.
// =============================================================================

use super::resolve;
use super::predicates;
use super::resolve::is_manifest_go_external;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup, Resolution};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

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

        // Import refs (`import "fmt"`, `import "mymodule/pkg"`): namespace
        // declarations rather than symbol references — classify with the
        // module path so they move out of unresolved_refs.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            return Some(import_path.to_string());
        }

        // Go built-in functions, types, and composite-type literals
        // (`[]string`, `map[string]int`, `[]*Foo`) are runtime/stdlib.
        if predicates::is_go_builtin(target) || predicates::is_go_composite_type(target) {
            return Some("builtin".to_string());
        }

        // Only exported (capitalized) names can come from external packages.
        let is_exported = target.chars().next().is_some_and(|c| c.is_uppercase());
        if !is_exported {
            return None;
        }

        // Walk the file's imports; prefer the longest matching external
        // module path (most specific). Manifest-driven via go.mod when
        // ProjectContext is available, fallback predicate otherwise.
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
        resolve::detect_flow_inner_with_lookup(file_ctx, ref_ctx, lookup)
    }

    fn build_file_context(
        &self,
        file: &crate::types::ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<crate::indexer::resolve::engine::FileContext> {
        Some(resolve::build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &crate::indexer::resolve::engine::FileContext,
        ref_ctx: &crate::indexer::resolve::engine::RefContext<'_>,
        lookup: &dyn crate::indexer::resolve::engine::SymbolLookup,
    ) -> Option<crate::indexer::resolve::engine::Resolution> {
        super::resolve::GoResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static GO_HOOKS: GoHooks = GoHooks;
