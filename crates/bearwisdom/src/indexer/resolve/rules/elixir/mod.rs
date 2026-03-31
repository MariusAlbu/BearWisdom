// =============================================================================
// indexer/resolve/rules/elixir/mod.rs — Elixir resolution rules
//
// Scope rules for Elixir:
//
//   1. Scope chain walk: innermost module/function → outermost.
//   2. Same-module resolution: functions defined in the same module are visible.
//   3. Alias resolution: `alias MyApp.Repo, as: Repo` brings Repo into scope.
//   4. Import resolution: `import Ecto.Query` brings query macros into scope.
//   5. Fully qualified module names: `MyApp.Accounts.User` resolves directly.
//
// Elixir module system:
//   `alias MyApp.Repo`               → shortens MyApp.Repo to Repo
//   `alias MyApp.Repo, as: R`        → shortens to R
//   `import Ecto.Query`              → makes functions from Ecto.Query visible
//   `use Phoenix.Controller`         → macro expansion, treated as import
//   `require Logger`                 → makes Logger macros available
//
// The extractor emits EdgeKind::Imports for all of alias/import/use/require:
//   target_name = the local alias or the module name
//   module      = the full module path
// =============================================================================

mod builtins;

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Elixir language resolver.
pub struct ElixirResolver;

impl LanguageResolver for ElixirResolver {
    fn language_ids(&self) -> &[&str] {
        &["elixir"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract the top-level module name as the file namespace.
        let file_namespace = file.symbols.iter().find_map(|sym| {
            if sym.kind == crate::types::SymbolKind::Namespace
                || sym.kind == crate::types::SymbolKind::Class
            {
                Some(sym.qualified_name.clone())
            } else {
                None
            }
        });

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let full_module = r.module.as_deref().unwrap_or(&r.target_name);

            // Determine the local binding name for this import:
            //   - If `module` is set, `target_name` is the local alias/binding.
            //   - If no `module`, `target_name` is the module itself; use last segment.
            let imported_name = if r.module.is_some() {
                r.target_name.clone()
            } else {
                // Default Elixir alias: last CamelCase segment.
                full_module
                    .split('.')
                    .last()
                    .unwrap_or(&r.target_name)
                    .to_string()
            };

            // Detect whether the local name differs from the last segment (i.e., `as:` was used).
            let last_segment = full_module.split('.').last().unwrap_or(full_module);
            let alias = if imported_name != last_segment {
                Some(imported_name.clone())
            } else {
                None
            };

            imports.push(ImportEntry {
                imported_name,
                module_path: Some(full_module.to_string()),
                alias,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "elixir".to_string(),
            imports,
            file_namespace,
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

        // Elixir builtins (Kernel functions, ExUnit macros, etc.).
        if builtins::is_elixir_builtin(target) {
            return None;
        }

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "elixir_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-module resolution.
        if let Some(module) = &file_ctx.file_namespace {
            let candidate = format!("{module}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "elixir_same_module",
                    });
                }
            }
        }

        // Step 3: Alias resolution — expand the target via known aliases.
        for import in &file_ctx.imports {
            if import.imported_name != *target {
                continue;
            }
            if let Some(full_module) = &import.module_path {
                if let Some(sym) = lookup.by_qualified_name(full_module) {
                    if builtins::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "elixir_alias",
                        });
                    }
                }
            }
        }

        // Step 4: Fully qualified module reference.
        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "elixir_qualified_name",
                    });
                }
            }
        }

        // Step 5: Simple name lookup.
        for sym in lookup.by_name(target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "elixir_by_name",
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import / alias / use / require directives.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            if builtins::is_external_elixir_module(module) {
                let root = module.split('.').next().unwrap_or(module);
                return Some(root.to_string());
            }
            return None;
        }

        // Elixir builtins (Kernel functions).
        if builtins::is_elixir_builtin(target) {
            return Some("Kernel".to_string());
        }

        // Check whether the target matches a known-external alias in this file.
        for import in &file_ctx.imports {
            if import.imported_name != *target {
                continue;
            }
            let module = import.module_path.as_deref().unwrap_or("");
            if builtins::is_external_elixir_module(module) {
                let root = module.split('.').next().unwrap_or(module);
                return Some(root.to_string());
            }
        }

        // Fully-qualified external module reference.
        if target.contains('.') {
            let root = target.split('.').next().unwrap_or(target);
            if builtins::is_external_elixir_module(root) {
                return Some(root.to_string());
            }
        } else {
            // Plain module name (single segment, uppercase = Elixir module).
            if builtins::is_external_elixir_module(target) {
                return Some(target.clone());
            }
        }

        None
    }
}
