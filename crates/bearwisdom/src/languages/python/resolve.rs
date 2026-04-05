// =============================================================================
// indexer/resolve/rules/python/mod.rs — Python resolution rules
//
// Scope rules for Python:
//
//   1. Chain-aware resolution: walk MemberChain step-by-step following
//      field types / return types.
//   2. Scope chain walk: innermost → outermost, try {scope}.{target}.
//   3. Same-file resolution: symbols defined in the same file are visible
//      at module scope without any import.
//   4. Import-based resolution: `from app.models import User` → `User` resolves.
//   5. Module-qualified: `models.User` → look up via imports.
//
// Python import forms:
//   `import os`               → import_name = "os",   module = None
//   `from foo import Bar`     → import_name = "Bar",  module = "foo"
//   `from foo.bar import Baz` → import_name = "Baz",  module = "foo.bar"
//   `import foo as f`         → import_name = "f",    module = "foo", alias = "f"
//
// The extractor emits EdgeKind::Imports for `import` and `from ... import`
// statements, with:
//   target_name = the bound local name (or module for bare `import`)
//   module      = the source module path (for `from ... import`)
//
// `self` is handled exactly like TypeScript's `this`: SelfRef segments
// trigger find_enclosing_class which walks the scope_chain for a class.
// =============================================================================


use super::{builtins, chain};
use crate::indexer::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use tracing::debug;

/// Python language resolver.
pub struct PythonResolver;

impl LanguageResolver for PythonResolver {
    fn language_ids(&self) -> &[&str] {
        &["python"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Collect import entries from EdgeKind::Imports refs.
        //
        // The Python extractor emits:
        //   `import os`
        //     → ref { target_name: "os", module: None,    kind: Imports }
        //   `from foo.bar import Baz`
        //     → ref { target_name: "Baz", module: "foo.bar", kind: Imports }
        //   `from . import something` (relative)
        //     → ref { target_name: "something", module: ".", kind: Imports }
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }

            // Bare `import os`: module_path = "os", imported_name = "os"
            // `from foo import Bar`: module_path = "foo", imported_name = "Bar"
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            let imported_name = r.target_name.clone();
            let is_wildcard = imported_name == "*";

            imports.push(ImportEntry {
                imported_name,
                module_path,
                alias: None,
                is_wildcard,
            });
        }

        // Python has no explicit file-level namespace — identity is the file path.
        FileContext {
            file_path: file.path.clone(),
            language: "python".to_string(),
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

        // Skip import refs.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Python builtins are never in our index.
        if builtins::is_python_builtin(target) {
            return None;
        }

        // Chain-aware resolution.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = chain::resolve_via_chain(chain_val, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // If the ref carries a module path, it came from an import statement.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if builtins::is_relative_import(module) {
                // Relative import — try to resolve in the target file.
                for sym in lookup.in_file(module) {
                    if sym.name == *target && builtins::kind_compatible(edge_kind, &sym.kind) {
                        debug!(
                            strategy = "python_import_file",
                            file = %module,
                            target = %target,
                            "resolved"
                        );
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "python_import_file",
                        });
                    }
                }

                let candidate = format!("{module}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if builtins::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "python_import",
                        });
                    }
                }
            }
            // External package or unresolvable — skip.
            return None;
        }

        // Strip `self.` prefix — `self.method` → `method`, scope_chain handles it.
        let effective_target = target.strip_prefix("self.").unwrap_or(target);

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "python_scope_chain",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && builtins::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "python_same_file",
                    qualified_name = %sym.qualified_name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "python_same_file",
                });
            }
        }

        // Step 3: Fully qualified name (dotted target like "models.User").
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_qualified_name",
                    });
                }
            }

            // Dotted: split into module alias + symbol, look up via imports.
            if let Some(dot) = effective_target.find('.') {
                let alias = &effective_target[..dot];
                let rest = &effective_target[dot + 1..];

                // Find the import whose imported_name matches the alias.
                for import in &file_ctx.imports {
                    if import.imported_name != alias {
                        continue;
                    }
                    let Some(ref mod_path) = import.module_path else {
                        continue;
                    };

                    let candidate = format!("{mod_path}.{rest}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if builtins::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "python_module_qualified",
                            });
                        }
                    }

                    // Also try searching by name within that module.
                    let method_name = rest.split('.').next().unwrap_or(rest);
                    for sym in lookup.by_name(method_name) {
                        if sym.qualified_name.starts_with(mod_path.as_str())
                            && builtins::kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "python_module_qualified_by_name",
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Import-based resolution for simple names.
        // `from app.models import User` → `User` resolves to `app.models.User`.
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(ref mod_path) = import.module_path {
                    if !builtins::is_relative_import(mod_path) {
                        continue;
                    }
                    let candidate = format!("{mod_path}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if builtins::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "python_wildcard_import",
                            });
                        }
                    }
                }
                continue;
            }

            if import.imported_name != effective_target {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };

            // `from foo.bar import Baz` → try `foo.bar.Baz`
            let candidate = format!("{mod_path}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "python_from_import",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_from_import",
                    });
                }
            }

            // Also search by simple name scoped to the module.
            for sym in lookup.by_name(effective_target) {
                if sym.qualified_name.starts_with(mod_path.as_str())
                    && builtins::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "python_from_import_prefix",
                    });
                }
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

        // Import refs: classify the module as external or stdlib.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            // Relative imports are internal.
            if builtins::is_relative_import(module) {
                return None;
            }
            let root = module.split('.').next().unwrap_or(module);
            if builtins::is_python_stdlib(root) {
                return Some("stdlib".to_string());
            }
            // Manifest-driven: check pyproject.toml / requirements.txt dependencies first.
            // pip package names may use hyphens; Python imports use underscores.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests.get(&ManifestKind::PyProject) {
                    if manifest.dependencies.contains(root)
                        || manifest.dependencies.contains(&root.replace('_', "-"))
                    {
                        return Some(module.to_string());
                    }
                }
            }
            let is_ext = match project_ctx {
                Some(ctx) => ctx.is_external_python_package(root),
                None => true,
            };
            if is_ext {
                return Some(module.to_string());
            }
            return None;
        }

        // Python builtins (built-in functions, types, and common method names).
        if builtins::is_python_builtin(target) {
            return Some("python_builtins".to_string());
        }

        // Walk file imports for a match.
        let simple = target.split('.').next().unwrap_or(target);
        for import in &file_ctx.imports {
            if import.imported_name != simple {
                continue;
            }
            let Some(ref mod_path) = import.module_path else {
                continue;
            };
            if builtins::is_relative_import(mod_path) {
                continue;
            }
            let root = mod_path.split('.').next().unwrap_or(mod_path);
            if builtins::is_python_stdlib(root) {
                return Some("stdlib".to_string());
            }
            // Manifest-driven check.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests.get(&ManifestKind::PyProject) {
                    if manifest.dependencies.contains(root)
                        || manifest.dependencies.contains(&root.replace('_', "-"))
                    {
                        return Some(mod_path.clone());
                    }
                }
            }
            let is_ext = match project_ctx {
                Some(ctx) => ctx.is_external_python_package(root),
                None => true,
            };
            if is_ext {
                return Some(mod_path.clone());
            }
        }

        None
    }

    // is_visible: default (always true). Python has no enforced access control
    // at runtime — `_private` is convention only and we don't track it.
}
