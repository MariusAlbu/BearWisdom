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


use super::{predicates, type_checker::PythonChecker};
use crate::type_checker::TypeChecker;
use crate::ecosystem::manifest::ManifestKind;
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

        // Bare-name walker lookup. cpython_stdlib emits real symbols for
        // `print`, `len`, `dict`, exception types, str/list/dict methods,
        // etc. under `ext:cpython-stdlib:`. Only bind to walker symbols
        // here — internal-name binding is handled more precisely by the
        // import-prefix and same-file paths below.
        if !target.contains('.') && !target.contains("::") {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "python_synthetic_global",
                    resolved_yield_type: None,
                });
            }
        }

        // Chain-aware resolution: dispatch to PythonChecker.
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = PythonChecker.resolve_chain(
                chain_val, edge_kind, None, ref_ctx, lookup,
            ) {
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
            if predicates::is_relative_import(module) {
                // Relative import — try to resolve in the target file.
                for sym in lookup.in_file(module) {
                    if sym.name == *target && predicates::kind_compatible(edge_kind, &sym.kind) {
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
                            resolved_yield_type: None,
                        });
                    }
                }

                let candidate = format!("{module}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "python_import",
                            resolved_yield_type: None,
                        });
                    }
                }
            } else {
                // Module-qualified ref: the extractor saw a `models.TextChoices`-
                // shaped reference and split it into target=`TextChoices`,
                // module=`models`. Two distinct sub-cases share the same
                // resolution machinery:
                //
                //   (B1) Chain-bearing call ref — `Person.objects.filter()`
                //        with the extractor's post-pass attaching the
                //        absolute module path.
                //   (B2) Module-qualified Inherits / TypeRef without a
                //        chain — `class Foo(models.TextChoices):`,
                //        `field: models.CharField`. The ref has the
                //        module attached but no member-chain because
                //        there's no further dispatch beyond the type
                //        access.
                //
                // Both shapes need the same lookup attempts. Previously
                // (B2) fell through to the chain-required `else` branch
                // and short-circuited as "unresolvable", which left every
                // Django `class Foo(models.TextChoices):` /
                // `IntegerChoices` / `CharField` ref unresolved despite
                // the symbols being in the externals index.
                let is_imports = ref_ctx.extracted_ref.kind == EdgeKind::Imports;
                if is_imports {
                    return None;
                }

                let candidate = format!("{module}.{target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        debug!(
                            strategy = "python_ref_module",
                            candidate = %candidate,
                            "resolved"
                        );
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "python_ref_module",
                            resolved_yield_type: None,
                        });
                    }
                }
                // Look up target by name in files whose path contains the
                // module path. Handles the dominant Django shape where
                // `models.TextChoices` lives at qname `TextChoices` in
                // `django/db/models/enums.py` rather than at
                // `django.db.models.TextChoices`.
                let module_as_path = module.replace('.', "/");
                for sym in lookup.by_name(target) {
                    if sym.file_path.contains(&module_as_path)
                        && predicates::kind_compatible(edge_kind, &sym.kind)
                    {
                        debug!(
                            strategy = "python_ref_module_path",
                            module_path = %module_as_path,
                            target = %target,
                            "resolved"
                        );
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "python_ref_module_path",
                            resolved_yield_type: None,
                        });
                    }
                }
                // Resolve the module against the file's import map. For
                // `class Foo(models.TextChoices)` where the file has
                // `from django.db import models`, walk `django/db/` for
                // a `TextChoices` symbol — same logic as the import-loop
                // below but threaded by the ref's own module attribute.
                for import in &file_ctx.imports {
                    if import.imported_name != *module {
                        continue;
                    }
                    let Some(ref base_mod) = import.module_path else { continue };
                    let base_dir = base_mod.replace('.', "/");
                    for sym in lookup.by_name(target) {
                        let norm = sym.file_path.replace('\\', "/");
                        let combined = format!("{base_dir}/{module_as_path}");
                        let in_dir = norm.contains(&combined)
                            || norm.contains(&format!("{base_dir}/{module}/"))
                            || norm.contains(&base_dir.as_str());
                        if in_dir && predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "python_ref_module_via_import",
                                resolved_yield_type: None,
                            });
                        }
                    }
                }
                // Case (B) miss — fall through to scope chain walk.
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
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "python_scope_chain",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_scope_chain",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && predicates::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "python_same_file",
                    qualified_name = %sym.qualified_name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "python_same_file",
                    resolved_yield_type: None,
                });
            }
        }

        // Step 3: Fully qualified name (dotted target like "models.User").
        if effective_target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(effective_target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_qualified_name",
                        resolved_yield_type: None,
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
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "python_module_qualified",
                                resolved_yield_type: None,
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
                        if in_mod && predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "python_module_qualified_by_name",
                                resolved_yield_type: None,
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
                    if !predicates::is_relative_import(mod_path) {
                        continue;
                    }
                    let candidate = format!("{mod_path}.{effective_target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if predicates::kind_compatible(edge_kind, &sym.kind) {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 0.90,
                                strategy: "python_wildcard_import",
                                resolved_yield_type: None,
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
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    debug!(
                        strategy = "python_from_import",
                        candidate = %candidate,
                        "resolved"
                    );
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "python_from_import",
                        resolved_yield_type: None,
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
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "python_from_import_prefix",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        // Python bare-name fallback for unittest-style assertion / mixin
        // calls. Counterpart to the SCSS / Bash bare-name steps. The chain
        // walker can't follow `self.assertEqual` through Django's deep
        // TestCase hierarchy (`APITestCase` → … → `unittest.TestCase`)
        // without inheritance type-flow, so chain refs that resolve to
        // a leaf method on `self` fall through here. Bind to any
        // Python-defined symbol whose simple name matches, gated by file
        // extension so cross-language collisions don't leak.
        //
        // Scoped to Calls/TypeRef: an `Imports` ref already short-circuits
        // above, and `Inherits` falls outside this leaf-method shape.
        //
        // TypeRef accepts `method` here because context-manager call
        // patterns like `with self.assertLogs(): …` are emitted as
        // TypeRef by the Python extractor (the `with`-target's type is
        // technically what's referenced), but the bound symbol is a
        // method on TestCase. Tightening to TypeRef = class-only would
        // miss every `with self.assert*` block on Django/DRF tests.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef)
            && ref_ctx.extracted_ref.module.is_none()
            && !effective_target.contains('.')
        {
            let kind_ok = |sym_kind: &str| -> bool {
                if predicates::kind_compatible(edge_kind, sym_kind) {
                    return true;
                }
                edge_kind == EdgeKind::TypeRef && sym_kind == "method"
            };
            for sym in lookup.by_name(effective_target) {
                if !kind_ok(&sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_py = path.ends_with(".py")
                    || path.ends_with(".pyi")
                    || path.starts_with("ext:python:")
                    || path.starts_with("ext:idx:");
                if !is_py {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "python_bare_name",
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
        infer_external_inner(file_ctx, ref_ctx, project_ctx, None)
    }

    fn infer_external_namespace_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    // is_visible: default (always true). Python has no enforced access control
    // at runtime — `_private` is convention only and we don't track it.
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Check whether a Python package root is an external dependency using the project manifest.
fn is_manifest_python_package(ctx: &ProjectContext, name: &str) -> bool {
    ctx.has_dependency(ManifestKind::PyProject, name)
        || ctx.has_dependency(ManifestKind::PyProject, &name.replace('_', "-"))
}

/// Returns Some(namespace) when `module` should be treated as external. Walks
/// the manifest, the stdlib list, and finally a `has_in_namespace` structural
/// check that catches transitive deps and stdlib-version gaps without growing
/// the hardcoded set (e.g. `httpx` pulled in via `httpx-oauth`, or `zoneinfo`
/// added in 3.9).
fn module_is_external(
    project_ctx: Option<&ProjectContext>,
    pkg_id: Option<i64>,
    lookup: Option<&dyn SymbolLookup>,
    module: &str,
) -> Option<String> {
    let root = module.split('.').next().unwrap_or(module);
    if let Some(ctx) = project_ctx {
        if let Some(manifest) = ctx
            .manifests_for(pkg_id)
            .get(&ManifestKind::PyProject)
        {
            if manifest.dependencies.contains(root)
                || manifest.dependencies.contains(&root.replace('_', "-"))
            {
                return Some(module.to_string());
            }
        }
        if is_manifest_python_package(ctx, root) {
            return Some(module.to_string());
        }
    }
    if let Some(lookup) = lookup {
        // No internal symbols under this module name → external (transitive
        // dep, package-rename, or runtime-only library).
        if !lookup.has_in_namespace(root) {
            return Some(module.to_string());
        }
    }
    if project_ctx.is_none() {
        // No manifest visible — be permissive (matches the prior behaviour).
        return Some(module.to_string());
    }
    None
}

fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;
    let pkg_id = ref_ctx.file_package_id;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        if predicates::is_relative_import(module) {
            return None;
        }
        return module_is_external(project_ctx, pkg_id, lookup, module);
    }

    let simple = target.split('.').next().unwrap_or(target);
    for import in &file_ctx.imports {
        if import.imported_name != simple {
            continue;
        }
        let Some(ref mod_path) = import.module_path else {
            continue;
        };
        if predicates::is_relative_import(mod_path) {
            continue;
        }
        if let Some(ns) = module_is_external(project_ctx, pkg_id, lookup, mod_path) {
            return Some(ns);
        }
    }
    None
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
            namespace_segments: Vec::new(),
            chain: None,
            byte_offset: 0,
        }
    }

    fn make_py_file(path: &str, syms: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "python".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            mtime: None,
            package_id: None,
            content: None,
            has_errors: false,
            symbols: syms,
            refs,
            routes: vec![],
            db_sets: vec![],
            symbol_origin_languages: vec![],
            ref_origin_languages: vec![],
            symbol_from_snippet: vec![],
            flow: crate::types::FlowMeta::default(),
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),
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
                mtime: None,
                package_id: None,
                content: None,
                has_errors: false,
                symbols: f.symbols.clone(),
                refs: f.refs.clone(),
                routes: vec![],
                db_sets: vec![],
                symbol_origin_languages: vec![],
                ref_origin_languages: vec![],
                symbol_from_snippet: vec![],
                flow: crate::types::FlowMeta::default(),
                connection_points: Vec::new(),
                demand_contributions: Vec::new(),
                alias_targets: Vec::new(),
            component_selectors: Vec::new(),
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
            byte_offset: 0,
                    namespace_segments: Vec::new(),
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
        file_package_id: None,
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
            byte_offset: 0,
                    namespace_segments: Vec::new(),
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
        file_package_id: None,
        };

        let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
        assert!(
            result.is_some(),
            "Team should resolve on Windows backslash paths"
        );
    }
}
