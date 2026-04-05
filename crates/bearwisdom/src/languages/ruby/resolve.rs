// =============================================================================
// indexer/resolve/rules/ruby/mod.rs — Ruby resolution rules
//
// Scope rules for Ruby:
//
//   1. Chain-aware resolution: walk MemberChain following field/return types.
//   2. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   3. Same-file / same-module resolution: Ruby classes/modules in the same
//      file are visible to each other at module scope.
//   4. Constant resolution: Ruby classes and modules are constants.
//      An unqualified `Foo` is looked up in the nesting chain (scope chain).
//
// Ruby import model:
//   The Ruby extractor emits EdgeKind::Imports refs for require statements:
//     require 'rails'          → target_name = "rails",   module = None
//     require_relative './foo' → target_name = "foo",     module = "./foo"
//
//   require gives access to library constants/classes by their top-level name
//   (e.g., requiring 'rails' makes `Rails::Application` available).
//   require_relative brings in local file symbols.
//
// Adding new Ruby features:
//   - autoload → add to build_file_context.
//   - include / extend (mixins) → update extractor to emit TypeRef with the
//     module name; this resolver picks them up via scope chain.
// =============================================================================


use super::{builtins, chain};
use crate::indexer::manifest::ManifestKind;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Ruby language resolver.
pub struct RubyResolver;

impl LanguageResolver for RubyResolver {
    fn language_ids(&self) -> &[&str] {
        &["ruby"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Extract require / require_relative imports from EdgeKind::Imports refs.
        // Ruby extractor emits:
        //   require 'foo'          → target_name = "foo",        module = None (bare gem)
        //   require_relative './x' → target_name = "x",          module = "./x" (relative)
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                is_wildcard: false,
            });
        }

        // Ruby has no file-level package/namespace declaration in the same sense —
        // classes are constants defined at file scope. The outermost module name
        // (if any) is extracted from the first Namespace/Module symbol.
        // Ruby modules are represented as Namespace in the SymbolKind enum.
        let file_namespace = file.symbols.iter().find_map(|sym| {
            if sym.kind == crate::types::SymbolKind::Namespace {
                Some(sym.qualified_name.clone())
            } else {
                None
            }
        });

        FileContext {
            file_path: file.path.clone(),
            language: "ruby".to_string(),
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

        // Skip import refs — they're not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Chain-aware resolution: if we have a structured MemberChain, walk it
        // step-by-step following field types.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = chain::resolve_via_chain(chain_val, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["MyModule::MyClass::my_method", "MyModule::MyClass", "MyModule"]
        // Try "MyModule::MyClass::my_method::Target", etc.
        // Note: Ruby uses "::" as the namespace separator in qualified names.
        // The index may store qualified names with either "." or "::" depending on the extractor.
        for scope in &ref_ctx.scope_chain {
            // Try dotted form (how the extractor stores qualified names).
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        // In Ruby, classes/methods in the same file are visible at module scope.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target && builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ruby_same_file",
                });
            }
        }

        // Step 3: Same-module resolution.
        // If we're inside a module, sibling constants are visible unqualified.
        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_same_module",
                    });
                }
            }
        }

        // Step 4: Fully qualified name (target contains "::" or ".").
        if target.contains("::") || target.contains('.') {
            // Normalize "::" to "." for index lookup.
            let normalized = target.replace("::", ".");
            if let Some(sym) = lookup.by_qualified_name(&normalized) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_qualified_name",
                    });
                }
            }
            if let Some(sym) = lookup.by_qualified_name(target) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_qualified_name",
                    });
                }
            }
        }

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Import refs (require statements) — classify the require itself if external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let require_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);

            // Manifest-driven: check Gemfile dependencies first.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests.get(&ManifestKind::Gemfile) {
                    let gem_root = require_path.split('/').next().unwrap_or(require_path);
                    if manifest.dependencies.contains(gem_root)
                        || manifest.dependencies.contains(require_path)
                    {
                        return Some(require_path.to_string());
                    }
                }
            }

            if builtins::is_external_ruby_require(require_path, project_ctx) {
                return Some(require_path.to_string());
            }
            return None;
        }

        // Ruby built-ins — always external.
        if builtins::is_ruby_builtin(target) {
            return Some("ruby_core".to_string());
        }

        // Check file's require list for matching external gems.
        // If the name was brought in by a gem require, it's external.
        for import in &file_ctx.imports {
            let Some(module_path) = &import.module_path else {
                continue;
            };

            // Only bare (non-relative) requires can be gems.
            if module_path.starts_with('.') {
                continue;
            }

            // Manifest-driven check.
            if let Some(ctx) = project_ctx {
                if let Some(manifest) = ctx.manifests.get(&ManifestKind::Gemfile) {
                    let gem_root = module_path.split('/').next().unwrap_or(module_path);
                    if manifest.dependencies.contains(gem_root)
                        || manifest.dependencies.contains(module_path.as_str())
                    {
                        return Some(module_path.clone());
                    }
                }
            }

            if builtins::is_external_ruby_require(module_path, project_ctx) {
                return Some(module_path.clone());
            }
        }

        None
    }

    // is_visible: default (always true) is correct for Ruby.
    // Ruby access control (private/protected) is enforced at runtime, not
    // at the call site — all indexed symbols are accessible for our purposes.
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

