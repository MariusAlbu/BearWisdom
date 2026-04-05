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

        // If the ref carries a module path, two distinct cases apply:
        //
        // (A) Import-statement refs (no chain): the module is the import source.
        //     If we can't resolve them here, there's nothing more to try — return None.
        //
        // (B) Call refs with a module set by the extractor post-pass (e.g.
        //     `Person.objects.filter()` → module="posthog.models"): use the module
        //     to locate the target before falling through to scope chain walk.
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
            } else if ref_ctx.extracted_ref.chain.is_some() {
                // Case (B): call ref with extractor-set module (absolute module path).
                // Try "{module}.{target}" as a qualified name.
                let candidate = format!("{module}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if builtins::kind_compatible(edge_kind, &sym.kind) {
                        debug!(
                            strategy = "python_ref_module",
                            candidate = %candidate,
                            "resolved"
                        );
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "python_ref_module",
                        });
                    }
                }
                // Try: look up target by name in files matching the module path.
                let module_as_path = module.replace('.', "/");
                for sym in lookup.by_name(target) {
                    if sym.file_path.contains(&module_as_path)
                        && builtins::kind_compatible(edge_kind, &sym.kind)
                    {
                        debug!(
                            strategy = "python_ref_module_path",
                            module_path = %module_as_path,
                            target = %target,
                            "resolved"
                        );
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "python_ref_module_path",
                        });
                    }
                }
                // Case (B) miss — fall through to scope chain walk.
            } else {
                // Case (A): import-statement ref, no chain — external or unresolvable.
                return None;
            }

            // Case (A) relative import that failed, or case (B) that fell through.
            // For case (A) relative failures we also stop here (no scope walk would help).
            if ref_ctx.extracted_ref.chain.is_none() {
                return None;
            }
            // Case (B) falls through to scope chain walk below.
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

                    // Also try searching by name within that module, including
                    // submodules (handles __init__.py re-exports).
                    let method_name = rest.split('.').next().unwrap_or(rest);
                    let mod_dir = mod_path.replace('.', "/");
                    for sym in lookup.by_name(method_name) {
                        let norm_path = sym.file_path.replace('\\', "/");
                        let in_mod = sym.qualified_name.starts_with(mod_path.as_str())
                            || norm_path.contains(&format!("{mod_dir}/"))
                            || norm_path.ends_with(&format!("/{mod_dir}.py"))
                            || norm_path == format!("{mod_dir}.py");
                        if in_mod && builtins::kind_compatible(edge_kind, &sym.kind) {
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
            // Two checks: qualified name prefix (works when the extractor embeds
            // the module path) OR file path under the module directory (handles
            // __init__.py re-exports where Person lives in posthog/models/person.py
            // but is imported as `from posthog.models import Person`).
            let module_dir = mod_path.replace('.', "/");
            for sym in lookup.by_name(effective_target) {
                let norm_path = sym.file_path.replace('\\', "/");
                let in_module_dir = norm_path.contains(&format!("{module_dir}/"))
                    || norm_path.ends_with(&format!("/{module_dir}.py"))
                    || norm_path == format!("{module_dir}.py");
                if (sym.qualified_name.starts_with(mod_path.as_str()) || in_module_dir)
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use crate::indexer::resolve::engine::{build_scope_chain, LanguageResolver, SymbolIndex};
    use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
    use std::collections::HashMap;

    fn make_sym(name: &str, qname: &str, kind: SymbolKind) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind,
            visibility: Some(Visibility::Public),
            start_line: 1,
            end_line: 10,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        }
    }

    fn make_import_ref(
        source_idx: usize,
        target: &str,
        module: &str,
        kind: EdgeKind,
    ) -> ExtractedRef {
        ExtractedRef {
            source_symbol_index: source_idx,
            target_name: target.to_string(),
            kind,
            line: 1,
            module: Some(module.to_string()),
            chain: None,
        }
    }

    fn make_py_file(path: &str, syms: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "python".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            content: None,
            has_errors: false,
            symbols: syms,
            refs,
            routes: vec![],
            db_sets: vec![],
        }
    }

    fn build_index(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
        let mut id_map = HashMap::new();
        let mut next_id = 1i64;
        for pf in files {
            for sym in &pf.symbols {
                id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
                next_id += 1;
            }
        }
        let owned: Vec<ParsedFile> = files
            .iter()
            .map(|f| ParsedFile {
                path: f.path.clone(),
                language: f.language.clone(),
                content_hash: String::new(),
                size: 0,
                line_count: 0,
                content: None,
                has_errors: false,
                symbols: f.symbols.clone(),
                refs: f.refs.clone(),
                routes: vec![],
                db_sets: vec![],
            })
            .collect();
        let index = SymbolIndex::build(&owned, &id_map);
        (index, id_map)
    }

    /// `from posthog.models import Person` in a consumer file should resolve `Person`
    /// to the class defined in `posthog/models/person.py`, even though `Person`'s
    /// qualified_name is just "Person" (not "posthog.models.person.Person").
    #[test]
    fn test_init_reexport_submodule_resolution() {
        // posthog/models/person.py defines Person
        let person_file = make_py_file(
            "posthog/models/person.py",
            vec![make_sym("Person", "Person", SymbolKind::Class)],
            vec![],
        );

        // posthog/api/views.py imports Person from posthog.models
        let consumer_sym = make_sym("get_person", "get_person", SymbolKind::Function);
        let import_ref = make_import_ref(0, "Person", "posthog.models", EdgeKind::Imports);
        let call_ref = ExtractedRef {
            source_symbol_index: 0,
            target_name: "Person".to_string(),
            kind: EdgeKind::Calls,
            line: 5,
            module: None,
            chain: None,
        };
        let consumer_file = make_py_file(
            "posthog/api/views.py",
            vec![consumer_sym],
            vec![import_ref, call_ref],
        );

        let (index, id_map) = build_index(&[&person_file, &consumer_file]);
        let resolver = PythonResolver;
        let file_ctx = resolver.build_file_context(&consumer_file, None);

        // The Calls ref to "Person" should resolve via python_from_import_prefix
        // (import says module=posthog.models, symbol lives under posthog/models/).
        let ref_ctx = RefContext {
            extracted_ref: &consumer_file.refs[1], // the Calls ref
            source_symbol: &consumer_file.symbols[0],
            scope_chain: build_scope_chain(None),
        };

        let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
        assert!(
            result.is_some(),
            "Person should resolve via __init__.py re-export path"
        );
        let res = result.unwrap();
        let expected_id = *id_map
            .get(&("posthog/models/person.py".to_string(), "Person".to_string()))
            .unwrap();
        assert_eq!(res.target_symbol_id, expected_id);
        assert_eq!(res.strategy, "python_from_import_prefix");
        assert!(res.confidence >= 0.95);
    }

    /// `from myapp.models import Team` where Team lives in `myapp/models/team.py`
    /// on Windows-style paths (backslash separators).
    #[test]
    fn test_init_reexport_windows_path() {
        let team_file = make_py_file(
            "myapp\\models\\team.py",
            vec![make_sym("Team", "Team", SymbolKind::Class)],
            vec![],
        );

        let consumer_sym = make_sym("handler", "handler", SymbolKind::Function);
        let import_ref = make_import_ref(0, "Team", "myapp.models", EdgeKind::Imports);
        let call_ref = ExtractedRef {
            source_symbol_index: 0,
            target_name: "Team".to_string(),
            kind: EdgeKind::TypeRef,
            line: 3,
            module: None,
            chain: None,
        };
        let consumer_file = make_py_file(
            "myapp\\api\\views.py",
            vec![consumer_sym],
            vec![import_ref, call_ref],
        );

        let (index, _) = build_index(&[&team_file, &consumer_file]);
        let resolver = PythonResolver;
        let file_ctx = resolver.build_file_context(&consumer_file, None);

        let ref_ctx = RefContext {
            extracted_ref: &consumer_file.refs[1],
            source_symbol: &consumer_file.symbols[0],
            scope_chain: build_scope_chain(None),
        };

        let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
        assert!(
            result.is_some(),
            "Team should resolve on Windows backslash paths"
        );
    }
}
