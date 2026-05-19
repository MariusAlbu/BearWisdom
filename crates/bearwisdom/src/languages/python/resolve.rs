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


use super::flow_detectors::{
    detect_python_bgjob_emission, detect_python_channels_consumer_inheritance,
    detect_python_channels_path_emission, detect_python_cursor_execute_emission,
    detect_python_db_query_emission, detect_python_django_path_emission,
    detect_python_graphql_decorator_emission, detect_python_grpc_stub_emission,
    detect_python_http_chain_emission, detect_python_mailer_emission,
    detect_python_redis_lookup, detect_python_route_decorator_emission,
    detect_python_sqlalchemy_select_call,
};
use super::externals::infer_external_inner;
use super::{predicates, type_checker::PythonChecker};
use crate::type_checker::TypeChecker;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};
use tracing::debug;

/// Python language resolver.
///
/// When the Python profile has `engine_primary` set, chain-bearing refs
/// route through `type_checker::Engine::resolve` before reaching this
/// resolver. This impl handles bare-name refs (imports, scope chain,
/// synthetic globals) and any chain refs the engine declines.
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
        // import-prefix and same-file paths below. Skip when the ref has
        // a chain — the chain walker's receiver-type context is more
        // precise than a bare-leaf lookup.
        if ref_ctx.extracted_ref.chain.is_none()
            && !target.contains('.') && !target.contains("::") {
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
                    flow_emit: None,
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
                            flow_emit: None,
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
                            flow_emit: None,
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
                            flow_emit: None,
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
                            flow_emit: None,
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
                                flow_emit: None,
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
                        flow_emit: None,
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
                    flow_emit: None,
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
                        flow_emit: None,
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
                                flow_emit: None,
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
                                flow_emit: None,
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
                                flow_emit: None,
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
                        flow_emit: None,
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
                        flow_emit: None,
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
                    flow_emit: None,
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

    fn detect_flow_emission(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let r = &ref_ctx.extracted_ref;

        // Decorator-based detection — TypeRef refs from extract_decorators
        // carry the decorator's dotted target name (`app.get`, `router.post`,
        // `app.route`) and the first string arg in `module`.
        if r.kind == EdgeKind::TypeRef {
            if let Some(emission) = detect_python_route_decorator_emission(
                r.target_name.as_str(),
                r.module.as_deref(),
            ) {
                return vec![emission];
            }
            // Django Channels: `class XConsumer(AsyncWebsocketConsumer)`
            // emits a TypeRef from the superclass list. Promote to a
            // single-ended Consumer WebSocket so the class clusters with
            // other WS endpoints.
            if let Some(emission) = detect_python_channels_consumer_inheritance(
                r.target_name.as_str(),
            ) {
                return vec![emission];
            }
            // Strawberry / Graphene GraphQL decorators: `@strawberry.type`,
            // `@strawberry.field`, `@strawberry.mutation`, `@strawberry.subscription`,
            // graphene `Schema(query=...)` / class-level `Query(graphene.ObjectType)`.
            if let Some(emission) = detect_python_graphql_decorator_emission(
                r.target_name.as_str(),
                ref_ctx.source_symbol.name.as_str(),
            ) {
                return vec![emission];
            }
            return Vec::new();
        }

        // Call-based detection — `requests.get('/x')`, `client.get('/x')`,
        // SQLAlchemy session.query(Entity), Django Entity.objects.filter,
        // FastAPI router-call-chain Producer.
        if r.kind != EdgeKind::Calls {
            return Vec::new();
        }

        // Django `path("users/", views.list)` / `re_path(...)` route
        // declarations land as Calls refs with no chain. Detection uses
        // target_name + call_args.
        if r.chain.is_none() {
            // Channels routing: when the file imports `channels`, a
            // `path("ws/x", X.as_asgi())` declares a WebSocket route, not
            // an HTTP one. Probed before the HTTP-path detector so the
            // emission is WS Consumer rather than HTTP Consumer.
            if let Some(emission) = detect_python_channels_path_emission(
                r.target_name.as_str(),
                &r.call_args,
                file_ctx,
            ) {
                return vec![emission];
            }
            if let Some(emission) = detect_python_django_path_emission(
                r.target_name.as_str(),
                &r.call_args,
                file_ctx,
            ) {
                return vec![emission];
            }
            // SQLAlchemy 2.x `select(Entity)` — bare call. Emits DbQuery Select.
            if let Some(emission) = detect_python_sqlalchemy_select_call(
                r.target_name.as_str(),
                &r.call_args,
                file_ctx,
            ) {
                return vec![emission];
            }
            return Vec::new();
        }
        let chain = r.chain.as_ref().unwrap();
        if let Some(emission) = detect_python_http_chain_emission(chain, &r.call_args, file_ctx) {
            return vec![emission];
        }
        if let Some(emission) = detect_python_db_query_emission(chain) {
            return vec![emission];
        }
        if let Some(emission) = detect_python_cursor_execute_emission(chain, &r.call_args) {
            return vec![emission];
        }
        if let Some(emission) = detect_python_grpc_stub_emission(chain) {
            return vec![emission];
        }
        if let Some(emission) = detect_python_mailer_emission(r.target_name.as_str(), chain) {
            return vec![emission];
        }
        if let Some(emission) = detect_python_bgjob_emission(chain) {
            return vec![emission];
        }
        if let Some(emission) = detect_python_redis_lookup(chain, &r.call_args) {
            return vec![emission];
        }
        Vec::new()
    }

    fn detect_flow_emission_with_lookup(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let direct = self.detect_flow_emission(file_ctx, ref_ctx);
        if !direct.is_empty() {
            return direct;
        }
        // Let-binding propagation: `stub = UserServiceStub(channel); stub.GetUser(req)`.
        let r = &ref_ctx.extracted_ref;
        let Some(chain) = r.chain.as_ref() else { return Vec::new() };
        let Some(root_seg) = chain.segments.first() else { return Vec::new() };
        if !matches!(root_seg.kind, crate::types::SegmentKind::Identifier) {
            return Vec::new();
        }
        let var_qname = match ref_ctx.source_symbol.scope_path.as_deref() {
            Some(scope) => format!("{}.{}", scope, root_seg.name),
            None => root_seg.name.clone(),
        };
        let type_name = match lookup.field_type_name(&var_qname) {
            Some(t) => t.to_string(),
            None => return Vec::new(),
        };
        if !type_name.ends_with("Stub") && !type_name.ends_with("Client") {
            return Vec::new();
        }
        let mut new_segments = vec![crate::types::ChainSegment {
            name: type_name,
            node_kind: "rewritten_var".to_string(),
            kind: crate::types::SegmentKind::Identifier,
            declared_type: None,
            type_args: vec![],
            optional_chaining: false,
            byte_offset: 0,
                    declared_type_id: None,
            type_arg_ids: Vec::new(),
}];
        new_segments.extend(chain.segments.iter().skip(1).cloned());
        let rewritten = crate::types::MemberChain { segments: new_segments };
        if let Some(em) = detect_python_grpc_stub_emission(&rewritten) {
            return vec![em];
        }
        Vec::new()
    }
}
