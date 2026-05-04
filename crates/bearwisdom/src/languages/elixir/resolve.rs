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


use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
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
        if predicates::is_elixir_builtin(target) {
            return None;
        }

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "elixir_scope_chain",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 2: Same-module resolution.
        if let Some(module) = &file_ctx.file_namespace {
            let candidate = format!("{module}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "elixir_same_module",
                        resolved_yield_type: None,
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
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "elixir_alias",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
        }

        // Step 4: Fully qualified module reference.
        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "elixir_qualified_name",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 5: Simple name lookup.
        for sym in lookup.by_name(target) {
            if predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "elixir_by_name",
                    resolved_yield_type: None,
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import / alias / use / require directives.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            let root = module.split('.').next().unwrap_or(module);

            // Manifest-driven: check mix.exs dependencies first.
            // Mix dep atoms are snake_case (e.g., "phoenix", "ecto_sql").
            // Elixir module roots are CamelCase (e.g., "Phoenix", "Ecto").
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Mix) {
                    if is_mix_dep_match(root, &manifest.dependencies) {
                        return Some(root.to_string());
                    }
                }
            }

            if predicates::is_external_elixir_module(module) {
                return Some(root.to_string());
            }
            return None;
        }

        // Elixir builtins (Kernel functions).
        if predicates::is_elixir_builtin(target) {
            return Some("Kernel".to_string());
        }

        // Check the ref's own module field (e.g., module="Ecto.Changeset"
        // on a type_ref to "Changeset" — the module IS the external package).
        if let Some(module) = &ref_ctx.extracted_ref.module {
            let root = module.split('.').next().unwrap_or(module);
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Mix) {
                    if is_mix_dep_match(root, &manifest.dependencies) {
                        return Some(root.to_string());
                    }
                }
            }
            if predicates::is_external_elixir_module(module) {
                return Some(root.to_string());
            }
        }

        // Check whether the target matches a known-external alias in this file.
        for import in &file_ctx.imports {
            if import.imported_name != *target {
                continue;
            }
            let module = import.module_path.as_deref().unwrap_or("");
            let root = module.split('.').next().unwrap_or(module);

            // Manifest-driven check for alias targets.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Mix) {
                    if is_mix_dep_match(root, &manifest.dependencies) {
                        return Some(root.to_string());
                    }
                }
            }

            if predicates::is_external_elixir_module(module) {
                return Some(root.to_string());
            }
        }

        // Fully-qualified external module reference.
        if target.contains('.') {
            let root = target.split('.').next().unwrap_or(target);

            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Mix) {
                    if is_mix_dep_match(root, &manifest.dependencies) {
                        return Some(root.to_string());
                    }
                }
            }

            if predicates::is_external_elixir_module(root) {
                return Some(root.to_string());
            }
        } else {
            // Plain module name (single segment, uppercase = Elixir module).
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Mix) {
                    if is_mix_dep_match(target, &manifest.dependencies) {
                        return Some(target.clone());
                    }
                }
            }

            if predicates::is_external_elixir_module(target) {
                return Some(target.clone());
            }
        }

        // `Routes` is the conventional alias for `<App>.Router.Helpers` — a
        // Phoenix compile-time module that never appears as a source-defined
        // symbol. It can reach files via multiple injection paths:
        //
        //   1. Direct alias: `alias MyApp.Router.Helpers, as: Routes` in source.
        //   2. ConnCase injection: handled by `is_phoenix_test_case_wrapper` below.
        //   3. Web wrapper injection: `use MyAppWeb, :controller` or `:view`
        //      — the web module's quote block injects the alias invisibly.
        //   4. Project-internal macro injection: `use MyApp.SomeResource` whose
        //      `defmacro __using__` quote block contains
        //      `alias PlausibleWeb.Router.Helpers, as: Routes`. The aliased
        //      `Routes` only takes effect in the using file, but its source
        //      `quote do` is invisible to BearWisdom.
        //
        // For case 1 we check for Router.Helpers in imports. For case 3 we
        // detect `use <AppWeb>, :<controller|view|...>` and apply externalization.
        // For case 4 we fall back to manifest evidence: if Phoenix is a declared
        // Mix dep AND the file has any `use ProjectInternal.Module` directive,
        // externalize. `Routes` is a universal Phoenix convention; an unresolved
        // single-segment `Routes` in a Phoenix project is essentially never a
        // user-defined non-Phoenix symbol.
        if target == "Routes" {
            for import in &file_ctx.imports {
                let mp = import.module_path.as_deref().unwrap_or("");
                // Case 1: explicit alias present in this file's own source.
                if mp.ends_with("Router.Helpers") {
                    return Some("Phoenix".to_string());
                }
                // Case 3: web wrapper module — `Routes` is injected by the
                // web module's `quote do` block; we can't see the alias itself.
                if predicates::is_internal_web_module(mp) {
                    return Some("Phoenix".to_string());
                }
            }
            // Case 4: manifest-evidence fallback. Activate only when the
            // project's mix.exs lists phoenix as a dependency AND the file
            // does at least one `use SomeProjectInternal.Module` (which is
            // the macro-injection vehicle). Files that don't `use` anything
            // can't have invisible aliases.
            let phoenix_in_mix = project_ctx
                .and_then(|ctx| {
                    ctx.manifests_for(ref_ctx.file_package_id)
                        .get(&ManifestKind::Mix)
                        .map(|m| is_mix_dep_match("phoenix", &m.dependencies))
                })
                .unwrap_or(false);
            let has_internal_use = file_ctx.imports.iter().any(|imp| {
                imp.module_path
                    .as_deref()
                    .map(|m| !m.is_empty()
                        && m.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
                    .unwrap_or(false)
            });
            if phoenix_in_mix && has_internal_use {
                return Some("Phoenix".to_string());
            }
        }

        // Use-macro injection inference: if the file has `use ExternalModule`
        // and that module is known to inject functions, any unresolved bare name
        // that matches the injection set is external. This is type inference
        // from the `use` statement — the `use` tells us what's available.
        for import in &file_ctx.imports {
            let module = import.module_path.as_deref().unwrap_or("");
            if module.is_empty() {
                continue;
            }

            // Phoenix test-case wrappers (e.g. `ChangelogWeb.ConnCase`) are
            // internal project modules, so the external-module guard below
            // would skip them. Handle them first, before that guard.
            //
            // These wrappers use `ExUnit.CaseTemplate` + `using do` blocks
            // that import `Phoenix.ConnTest` and alias `Router.Helpers` as
            // `Routes`. BearWisdom can't expand macros, so we detect the
            // wrapper by name convention and apply the ConnTest injection set.
            if predicates::is_phoenix_test_case_wrapper(module)
                && predicates::is_conn_case_injected(target)
            {
                return Some("Phoenix".to_string());
            }

            // Project-internal Schema modules (e.g. `Changelog.Schema`,
            // `MyApp.Schema`) commonly inject query-builder helpers via
            // `defmacro __using__` + `quote do`. BearWisdom can't expand
            // these macros, so the injected functions never appear as
            // top-level symbols. Detect by module name convention and apply
            // the known injection set.
            if predicates::is_internal_schema_module(module)
                && predicates::is_schema_using_injected(target)
            {
                // Attribute to the module's namespace root as an internal
                // origin (no external package). Use the full module path as
                // the namespace so the ref is classified as internal but
                // resolvable.
                return Some(module.split('.').next().unwrap_or(module).to_string());
            }

            // Project-internal `<AppWeb>` controller wrapper modules inject
            // shared helpers via a `def controller do quote do ... end end`
            // pattern. Any controller that does `use ChangelogWeb, :controller`
            // gets these helpers without them appearing as defined symbols.
            if predicates::is_internal_web_module(module)
                && predicates::is_web_controller_injected(target)
            {
                return Some(module.split('.').next().unwrap_or(module).to_string());
            }

            // Only check modules confirmed as external dependencies.
            let root = module.split('.').next().unwrap_or(module);
            let is_external_module = if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Mix) {
                    is_mix_dep_match(root, &manifest.dependencies)
                } else {
                    predicates::is_external_elixir_module(module)
                }
            } else {
                predicates::is_external_elixir_module(module)
            };
            if !is_external_module {
                continue;
            }
            // Check if this external module injects the target name via `use`.
            if predicates::is_use_injected(module, target) {
                return Some(root.to_string());
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a CamelCase Elixir module root matches any mix.exs dependency atom.
///
/// Mix dep atoms are snake_case (e.g., `"phoenix"`, `"ecto_sql"`).
/// Elixir module roots are CamelCase (e.g., `"Phoenix"`, `"Ecto"`).
///
/// Matching strategy:
/// 1. Lowercase the module root and compare directly (`"Phoenix"` → `"phoenix"`).
/// 2. Take the first underscore-separated segment of the dep atom and compare
///    (`"ecto_sql"` → `"ecto"`, which matches `"Ecto"`).
fn is_mix_dep_match(
    module_root: &str,
    deps: &std::collections::HashSet<String>,
) -> bool {
    let root_lower = module_root.to_lowercase();
    for dep in deps {
        // Direct lowercase match: "Phoenix" → "phoenix".
        if dep == &root_lower {
            return true;
        }
        // Prefix match: "ecto_sql" has root "ecto" which matches "Ecto".
        if let Some(prefix) = dep.split('_').next() {
            if prefix == root_lower {
                return true;
            }
        }
    }
    false
}
