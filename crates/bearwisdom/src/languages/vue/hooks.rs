use super::global_registry;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::ParsedFile;

pub struct VueHooks;

impl LanguageEngineHooks for VueHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        // Suppress external classification for explicitly globally-registered
        // PascalCase components (app.component) — these resolve by name in the
        // project index.
        if let Some(ctx_ref) = project_ctx {
            if let Some(registry) = ctx_ref
                .plugin_state
                .get::<global_registry::VueGlobalRegistry>()
            {
                let name = &ref_ctx.extracted_ref.target_name;
                if name.chars().next().map_or(false, |c| c.is_uppercase()) {
                    if let Some(global_registry::VueComponentSource::ExplicitRegistration {
                        ..
                    }) = registry.components.get(name.as_str())
                    {
                        return None;
                    }
                }
            }
        }
        crate::languages::typescript::hooks::infer_external_inner_with_lookup(
            file_ctx,
            ref_ctx,
            project_ctx,
            lookup,
        )
    }

    /// Build the per-file import table for a `.vue` file. The `<script>` block
    /// IS TypeScript, so the imports come from the shared TS file-context
    /// builder. On top of that, inject synthetic import entries for globally-
    /// registered Vue components so the import-driven engine strategies resolve
    /// `ComponentName` → `package.ComponentName` without an explicit import.
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut ctx =
            crate::languages::typescript::hooks::build_file_context_inner(file, project_ctx);

        // For each ref that doesn't already appear in the file's import list,
        // consult the project-wide global registry in plugin_state and add a
        // synthetic ImportEntry so the engine's import loop resolves it:
        //   1. exact-name binding from a generated `components.d.ts` /
        //      `auto-imports.d.ts` — the module is pinned by project config, so
        //      both PascalCase tags and camelCase composables bind verbatim;
        //   2. PascalCase tag covered by a library prefix convention.
        if let Some(ctx_ref) = project_ctx {
            if let Some(registry) = ctx_ref
                .plugin_state
                .get::<global_registry::VueGlobalRegistry>()
            {
                if !registry.is_empty() {
                    let already_imported: std::collections::HashSet<&str> = ctx
                        .imports
                        .iter()
                        .map(|e| e.imported_name.as_str())
                        .collect();

                    let mut extra_imports: Vec<ImportEntry> = Vec::new();
                    for r in &file.refs {
                        let name = &r.target_name;
                        if already_imported.contains(name.as_str()) {
                            continue;
                        }
                        if extra_imports.iter().any(|e| &e.imported_name == name) {
                            continue;
                        }
                        // Config-pinned exact module (components.d.ts /
                        // auto-imports.d.ts) wins — any identifier case.
                        let module = if let Some(m) =
                            global_registry::module_path_for(registry, name)
                        {
                            Some(m.to_string())
                        } else if name.chars().next().map_or(false, |c| c.is_uppercase()) {
                            // Library prefix convention — PascalCase tags only.
                            global_registry::library_for_name(registry, name).map(str::to_string)
                        } else {
                            None
                        };
                        if let Some(module_path) = module {
                            extra_imports.push(ImportEntry {
                                imported_name: name.clone(),
                                module_path: Some(module_path),
                                alias: None,
                                is_wildcard: false,
                            });
                        }
                    }
                    if !extra_imports.is_empty() {
                        ctx.imports.extend(extra_imports);
                    }
                }
            }
        }

        Some(ctx)
    }
}

pub static VUE_HOOKS: VueHooks = VueHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
