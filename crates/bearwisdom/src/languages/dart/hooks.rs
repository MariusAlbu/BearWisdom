use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use super::resolve;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, RefContext, SymbolLookup, Resolution};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::EdgeKind;

pub struct DartHooks;

impl LanguageEngineHooks for DartHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs — classify the URI.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let uri = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            if predicates::is_external_dart_import(uri) {
                let ns = if uri.starts_with("dart:") {
                    "dart.stdlib"
                } else if let Some(pkg_path) = uri.strip_prefix("package:") {
                    pkg_path.split('/').next().unwrap_or(pkg_path)
                } else {
                    uri
                };
                return Some(ns.to_string());
            }
            if uri.starts_with("package:") {
                if let Some(ctx) = project_ctx {
                    if let Some(manifest) = ctx
                        .manifests_for(ref_ctx.file_package_id)
                        .get(&ManifestKind::Pubspec)
                    {
                        let pkg_name = uri
                            .strip_prefix("package:")
                            .unwrap_or(uri)
                            .split('/')
                            .next()
                            .unwrap_or(uri);
                        if manifest.dependencies.contains(pkg_name) {
                            return Some(pkg_name.to_string());
                        }
                    }
                }
            }
            return None;
        }

        // Walk imports: if target was imported from an external URI, classify it.
        let simple = target.split('.').next().unwrap_or(target);
        for import in &file_ctx.imports {
            let uri = import.module_path.as_deref().unwrap_or("");
            if uri.is_empty() {
                continue;
            }

            let pkg_name_from_uri = if uri.starts_with("package:") {
                uri.strip_prefix("package:")
                    .unwrap_or(uri)
                    .split('/')
                    .next()
                    .unwrap_or(uri)
            } else {
                ""
            };

            let is_manifest_external = !pkg_name_from_uri.is_empty()
                && project_ctx
                    .and_then(|ctx| {
                        ctx.manifests_for(ref_ctx.file_package_id)
                            .get(&ManifestKind::Pubspec)
                    })
                    .is_some_and(|m| m.dependencies.contains(pkg_name_from_uri));

            // Alias-qualified: `u.Foo` where `import '...' as u`.
            if let Some(alias) = &import.alias {
                if alias == simple
                    && (is_manifest_external || predicates::is_external_dart_import(uri))
                {
                    if uri.starts_with("package:") {
                        return Some(pkg_name_from_uri.to_string());
                    }
                    return Some(uri.to_string());
                }
            }
            // Wildcard: any name could come from any non-aliased external import.
            if import.alias.is_none() {
                if is_manifest_external {
                    return Some(pkg_name_from_uri.to_string());
                }
                if predicates::is_external_dart_import(uri) {
                    if uri.starts_with("package:") {
                        return Some(pkg_name_from_uri.to_string());
                    }
                    return Some("dart.stdlib".to_string());
                }
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

    fn resolve_ref(
        &self,
        file_ctx: &crate::indexer::resolve::engine::FileContext,
        ref_ctx: &crate::indexer::resolve::engine::RefContext<'_>,
        lookup: &dyn crate::indexer::resolve::engine::SymbolLookup,
    ) -> Option<crate::indexer::resolve::engine::Resolution> {
        super::resolve::DartResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static DART_HOOKS: DartHooks = DartHooks;
