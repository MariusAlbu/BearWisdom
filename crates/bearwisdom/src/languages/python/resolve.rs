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
        }];
        new_segments.extend(chain.segments.iter().skip(1).cloned());
        let rewritten = crate::types::MemberChain { segments: new_segments };
        if let Some(em) = detect_python_grpc_stub_emission(&rewritten) {
            return vec![em];
        }
        Vec::new()
    }
}

pub(crate) fn detect_python_redis_lookup(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !matches!(root, "redis" | "r" | "cache" | "memcached" | "mc") {
        return None;
    }
    if !matches!(leaf, "get" | "hget" | "mget" | "getex") {
        return None;
    }
    let key = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })?;
    Some(FlowEmission::ConfigLookup { key: format!("redis:{}", key) })
}

pub(crate) fn detect_python_bgjob_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    // Celery: `task.delay(...)` / `task.apply_async(...)`.
    // RQ: `queue.enqueue(...)`.
    // Dramatiq: `task.send(...)`.
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let root = segs[0].name.as_str();
    let is_bg = matches!(leaf, "delay" | "apply_async" | "send_with_options")
        || (leaf == "enqueue" && (root == "queue" || root.ends_with("Queue") || root.ends_with("queue")));
    if !is_bg {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: format!("py.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

pub(crate) fn detect_python_mailer_emission(
    target_name: &str,
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    // Django: `send_mail(...)`, `EmailMessage(...).send()`, `mail.send_mass_mail`.
    // Flask-Mail: `mail.send(msg)`. Generic SMTP: `smtp.send_message`.
    let leaf = chain.segments.last().map(|s| s.name.as_str()).unwrap_or(target_name);
    if !matches!(leaf, "send" | "send_message" | "send_mail" | "send_mass_mail" | "send_html_mail") {
        return None;
    }
    // Restrict to chains whose root looks mail-related.
    let root = chain.segments.first()?.name.as_str();
    if !matches!(
        root,
        "mail" | "Mail" | "EmailMessage" | "EmailMultiAlternatives" | "smtp" | "smtplib"
    ) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("py.{}", root),
        role: ChannelRole::Producer,
        method: None,
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// HTTP Consumer — route decorators (FastAPI / Flask / Django path())
// ---------------------------------------------------------------------------

/// Recognise route decorators and emit Consumer HttpCall:
/// - FastAPI: `@app.get('/x')`, `@router.post('/x')`, also
///   `@app.put`/`patch`/`delete`/`head`/`options`/`api_route`. The decorator
///   extractor stores the dotted name (`app.get`) in `target_name` so the
///   trailing segment is the HTTP verb.
/// - Flask: `@app.route('/x', methods=['GET'])` — the verb defaults to GET
///   and lives in the optional `methods=` kwarg which we can't see from
///   the decorator first-arg, so emit with method=Any.
pub(crate) fn detect_python_route_decorator_emission(
    target_name: &str,
    first_arg: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    let path = first_arg?.trim();
    if path.is_empty() {
        return None;
    }
    let verb_seg = target_name.rsplit('.').next().unwrap_or(target_name);
    // WebSocket consumer: @app.websocket("/ws") / @router.websocket("/ws").
    if verb_seg == "websocket" {
        let name = crate::connectors::url_pattern::normalize(path);
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::WebSocket,
            name,
            role: ChannelRole::Consumer,
            method: None,
        streaming: None,
        });
    }
    let method = match verb_seg {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        // Flask `@app.route` and FastAPI `@app.api_route` accept any method
        // — the verb list lives in a kwarg we don't currently parse.
        "route" | "api_route" => HttpMethod::Any,
        _ => return None,
    };
    let name = crate::connectors::url_pattern::normalize(path);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(method),
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// HTTP Producer — requests / httpx / aiohttp / generic client.get('/x')
// ---------------------------------------------------------------------------

/// Recognise common Python HTTP client call shapes and emit Producer
/// HttpCall. Shapes handled (all leaf-segment-named verb + first
/// string-literal arg):
/// - `requests.get('/x')` / `requests.post(...)` / etc.
/// - `httpx.get(...)`, `httpx.AsyncClient().get(...)`, `client.get(...)`
/// - `aiohttp.ClientSession.get(...)`, `session.get(...)`
///
/// The detector fires when the chain leaf is a known HTTP verb AND the
/// file imports a recognised HTTP client library (so generic
/// `obj.get(key)` shapes like dict accessors don't misfire). The chain
/// root name doesn't need to match the library — long-lived session
/// bindings (`session = httpx.Client(); session.get('/x')`) just work via
/// the file's import set.
pub(crate) fn detect_python_http_chain_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();
    let method = match leaf {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        "head" => HttpMethod::Head,
        "options" => HttpMethod::Options,
        "request" => HttpMethod::Any,
        // urllib `urlopen(url)` / `urllib.request.urlopen(url)`.
        "urlopen" => HttpMethod::Any,
        _ => return None,
    };
    if !file_imports_python_http_library(file_ctx) {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.clone(),
        _ => return None,
    };
    if url.is_empty() {
        return None;
    }
    if !(url.starts_with('/') || url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(&url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Producer,
        method: Some(method),
    streaming: None,
    })
}

fn file_imports_python_http_library(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp.module_path.as_deref().unwrap_or(imp.imported_name.as_str());
        matches!(
            m.split('.').next().unwrap_or(m),
            "requests" | "httpx" | "aiohttp" | "urllib3" | "urllib" | "fastapi"
        )
    })
}

fn file_imports_python_django(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp.module_path.as_deref().unwrap_or(imp.imported_name.as_str());
        m.split('.').next().unwrap_or(m) == "django"
    })
}

pub(crate) fn file_imports_python_channels(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp.module_path.as_deref().unwrap_or(imp.imported_name.as_str());
        m.split('.').next().unwrap_or(m) == "channels"
    })
}

fn file_imports_python_sqlalchemy(file_ctx: &FileContext) -> bool {
    file_ctx.imports.iter().any(|imp| {
        let m = imp.module_path.as_deref().unwrap_or(imp.imported_name.as_str());
        let root = m.split('.').next().unwrap_or(m);
        root == "sqlalchemy" || root == "sqlmodel"
    })
}

// ---------------------------------------------------------------------------
// Django `path()` / `re_path()` routes → Consumer HttpCall
// ---------------------------------------------------------------------------

/// Detect Django URL conf entries:
///   `path("users/<int:id>/", views.detail)`
///   `re_path(r"^users/(?P<id>\d+)/$", views.detail)`
/// Both emit a Consumer HttpCall with method=Any (Django path entries
/// match any verb; per-method filtering lives inside the view).
/// Strawberry / Graphene GraphQL decorators. Recognises:
/// - `@strawberry.field` / `@strawberry.mutation` / `@strawberry.subscription`
///   on a resolver method → Consumer GraphQLOp keyed on the method name.
/// - `@strawberry.type` / `@strawberry.input` / `@strawberry.interface` on
///   a class → emits a DbEntity-style marker via the existing decorator
///   path. Schema-typed entities aren't paired today, so we limit
///   emission to the operation decorators where we have a method name.
pub(crate) fn detect_python_graphql_decorator_emission(
    target_name: &str,
    source_symbol_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let last = target_name.rsplit('.').next().unwrap_or(target_name);
    if !matches!(last, "field" | "mutation" | "subscription") {
        return None;
    }
    // The root of the dotted decorator must be a graphql library hint.
    let root = target_name.split('.').next().unwrap_or(target_name);
    if !matches!(root, "strawberry" | "graphene") {
        return None;
    }
    if source_symbol_name.is_empty() {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::GraphQLOp,
        name: format!("{}:{}", last, source_symbol_name),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

/// Django Channels consumer inheritance: any of the WebSocket consumer
/// base classes triggers a single-ended Consumer WebSocket emission for
/// the subclass.
pub(crate) fn detect_python_channels_consumer_inheritance(
    target_name: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let base = target_name.rsplit('.').next().unwrap_or(target_name);
    if !matches!(
        base,
        "WebsocketConsumer"
            | "AsyncWebsocketConsumer"
            | "JsonWebsocketConsumer"
            | "AsyncJsonWebsocketConsumer"
    ) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: "py.channels".to_string(),
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

/// Django Channels routing.py: `path("ws/x", X.as_asgi())` declares a
/// WebSocket route. Only fires when the file imports `channels.*` —
/// otherwise the call routes through the regular HTTP `path` detector.
pub(crate) fn detect_python_channels_path_emission(
    target_name: &str,
    call_args: &[crate::types::CallArg],
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    use crate::types::CallArg;
    if !matches!(target_name, "path" | "re_path") {
        return None;
    }
    if !file_imports_python_channels(file_ctx) {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.clone(),
        _ => return None,
    };
    if url.is_empty() {
        return None;
    }
    let cleaned: String = url
        .trim_start_matches('^')
        .trim_end_matches('$')
        .to_string();
    let name = crate::connectors::url_pattern::normalize(&cleaned);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name,
        role: ChannelRole::Consumer,
        method: None,
    streaming: None,
    })
}

pub(crate) fn detect_python_django_path_emission(
    target_name: &str,
    call_args: &[crate::types::CallArg],
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;

    if !matches!(target_name, "path" | "re_path") {
        return None;
    }
    if !file_imports_python_django(file_ctx) {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.clone(),
        _ => return None,
    };
    if url.is_empty() {
        return None;
    }
    // Normalise leading `^` and trailing `$` from re_path regex anchors so
    // the pattern compares cleanly against Producer-side URLs.
    let cleaned: String = url
        .trim_start_matches('^')
        .trim_end_matches('$')
        .to_string();
    let name = crate::connectors::url_pattern::normalize(&cleaned);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(HttpMethod::Any),
    streaming: None,
    })
}

// ---------------------------------------------------------------------------
// SQLAlchemy 2.x `select(Entity)` — bare-call DbQuery
// ---------------------------------------------------------------------------

/// Detect `select(User)` / `delete(User)` / `update(User)` / `insert(User)`
/// from SQLAlchemy 2.x where the entity name is the first positional arg.
/// Fires only when the file imports sqlalchemy or sqlmodel to avoid
/// matching unrelated `select` / `update` helper functions.
pub(crate) fn detect_python_sqlalchemy_select_call(
    target_name: &str,
    call_args: &[crate::types::CallArg],
    file_ctx: &FileContext,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;

    let op = match target_name {
        "select" => DbQueryOp::Select,
        "insert" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    if !file_imports_python_sqlalchemy(file_ctx) {
        return None;
    }
    let entity = call_args.iter().find_map(|a| match a {
        CallArg::Ident(s) if is_pascal_case_first(s) => Some(s.clone()),
        _ => None,
    })?;
    Some(FlowEmission::DbQuery {
        entity_name: namespaced_python_entity(&entity),
        operation: op,
    })
}

// ---------------------------------------------------------------------------
// Raw `cursor.execute("SELECT ...")` → DbQuery
// ---------------------------------------------------------------------------

/// Detect raw DB-API `cursor.execute("SELECT ...")` / `connection.execute(...)`
/// where the first arg is a SQL string. Entity is parsed from the FROM /
/// UPDATE / INSERT INTO / DELETE FROM clause.
pub(crate) fn detect_python_cursor_execute_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    use crate::types::CallArg;

    let leaf = chain.segments.last()?.name.as_str();
    if !matches!(leaf, "execute" | "executemany" | "executescript") {
        return None;
    }
    // Avoid matching SQLAlchemy `session.execute(select(User))` which is
    // detected on the inner `select` call instead — require the first arg
    // to be a StringLit for the raw DB-API path.
    let sql = match call_args.first()? {
        CallArg::StringLit(s) => s.as_str(),
        _ => return None,
    };
    let (entity, op) = parse_python_sql_entity(sql)?;
    Some(FlowEmission::DbQuery {
        entity_name: namespaced_python_entity(&entity),
        operation: op,
    })
}

fn parse_python_sql_entity(
    sql: &str,
) -> Option<(String, crate::indexer::resolve::flow_emit::DbQueryOp)> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    let upper = sql.trim().to_ascii_uppercase();
    let (op, after) = if let Some(i) = upper.find("INSERT INTO ") {
        (DbQueryOp::Insert, i + 12)
    } else if let Some(i) = upper.find("UPDATE ") {
        (DbQueryOp::Update, i + 7)
    } else if let Some(i) = upper.find("DELETE FROM ") {
        (DbQueryOp::Delete, i + 12)
    } else if let Some(i) = upper.find(" FROM ") {
        (DbQueryOp::Select, i + 6)
    } else if upper.starts_with("FROM ") {
        (DbQueryOp::Select, 5)
    } else {
        return None;
    };
    let slice = sql.get(after..)?;
    let token = slice.split_whitespace().next()?;
    let entity: String = token
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .to_string();
    if entity.is_empty() {
        return None;
    }
    Some((
        entity
            .rsplit('.')
            .next()
            .unwrap_or(entity.as_str())
            .to_string(),
        op,
    ))
}

// ---------------------------------------------------------------------------
// grpc-python Stub → RpcCall Producer
// ---------------------------------------------------------------------------

/// Detect `<Service>Stub(channel).Method(req)` — chain root is a
/// PascalCase identifier ending in `Stub` followed by the rpc method.
pub(crate) fn detect_python_grpc_stub_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !root.ends_with("Stub") || root == "Stub" {
        return None;
    }
    if !is_pascal_case_first(root) {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    // Skip when leaf is itself the constructor — `Stub(channel)` alone
    // shouldn't emit; only `Stub(channel).Method(...)` patterns do.
    if leaf == root {
        return None;
    }
    let service = root.strip_suffix("Stub").unwrap_or(root);
    let name = format!("{}.{}", service, leaf);
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name,
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
    })
}

// ---------------------------------------------------------------------------
// DbQuery — SQLAlchemy / Django ORM
// ---------------------------------------------------------------------------

/// Recognise DbQuery shapes and emit a single-ended DbQuery emission.
/// Shapes handled:
/// - SQLAlchemy `session.query(Entity)` → entity from the first chain
///   segment after `query` (none recorded — the query subject is the
///   call arg, not part of the chain in this AST).
/// - SQLAlchemy `Entity.query.filter(...)` (Flask-SQLAlchemy style) →
///   entity is the chain root.
/// - SQLAlchemy 2.x `select(Entity).where(...)` — entity is the call arg
///   of `select`; not recoverable from chain alone, skipped.
/// - Django `Entity.objects.filter(...)` / `Entity.objects.get(...)` /
///   `Entity.objects.create(...)` → entity is the chain root.
pub(crate) fn detect_python_db_query_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    let segs = &chain.segments;
    // Require exactly `[Entity, manager, op]` — three segments, with the
    // ORM op as the immediate leaf. Longer chains
    // (`Entity.objects.filter(x).order_by(y)`) emit their own
    // shorter-prefix Calls ref via the per-call_expression visitor; the
    // outermost ref carries the full chain and re-emitting on it would
    // multiply by chain depth. Capping at len==3 ensures one DbQuery
    // emission per Django/SQLAlchemy entity-access site.
    if segs.len() != 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    let mid = segs[1].name.as_str();
    let leaf = segs[2].name.as_str();
    if !is_pascal_case_first(root) {
        return None;
    }

    // Flask-SQLAlchemy `Entity.query.<op>` — `Entity` is the model class.
    if mid == "query" {
        let op = sqlalchemy_op_from_leaf(leaf)?;
        return Some(FlowEmission::DbQuery {
            entity_name: namespaced_python_entity(root),
            operation: op,
        });
    }

    // Django ORM `Entity.objects.<op>` — `Entity` is the model class.
    if mid == "objects" {
        let op = django_op_from_leaf(leaf)?;
        return Some(FlowEmission::DbQuery {
            entity_name: namespaced_python_entity(root),
            operation: op,
        });
    }

    None
}

/// Prefix the entity name with `py.` so the pairer's loose
/// `entity_names_match` (case-insensitive + pluralization tolerance)
/// doesn't cross-pair Python `Document.objects.X` queries with TypeScript
/// `@Document` decorators (Mongoose, NestJS) that produce a `DbEntity` for
/// the same bare name. The cost is that until a Python-side DbEntity
/// emission ships, these DbQuery rows stay single-ended — which is the
/// correct behaviour given the lack of a Python ORM-model FlowEmission
/// today.
fn namespaced_python_entity(name: &str) -> String {
    format!("py.{}", name)
}

fn sqlalchemy_op_from_leaf(
    name: &str,
) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        "filter" | "filter_by" | "first" | "all" | "one" | "one_or_none" | "scalar" | "get"
        | "count" | "exists" => DbQueryOp::Select,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        "add" | "insert" => DbQueryOp::Insert,
        _ => return None,
    })
}

fn django_op_from_leaf(
    name: &str,
) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        "filter" | "all" | "get" | "exclude" | "first" | "last" | "exists" | "count" | "values"
        | "values_list" => DbQueryOp::Select,
        "create" | "bulk_create" => DbQueryOp::Insert,
        "update" | "update_or_create" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        "get_or_create" => DbQueryOp::Upsert,
        _ => return None,
    })
}

fn is_pascal_case_first(name: &str) -> bool {
    name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
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
            call_args: Vec::new(),
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
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
            component_selectors: Vec::new(),

            plugin_flow_emissions: Vec::new(),
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
                demand_contributions: Vec::new(),
                alias_targets: Vec::new(),
            component_selectors: Vec::new(),

            plugin_flow_emissions: Vec::new(),
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
                    call_args: Vec::new(),
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
                    call_args: Vec::new(),
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

#[cfg(test)]
mod flow_tests {
    use super::*;
    use crate::types::{CallArg, ChainSegment, MemberChain, SegmentKind};

    fn make_chain(segments: &[&str]) -> MemberChain {
        MemberChain {
            segments: segments
                .iter()
                .enumerate()
                .map(|(i, name)| ChainSegment {
                    name: name.to_string(),
                    node_kind: if i == 0 { "identifier".to_string() } else { "property_identifier".to_string() },
                    kind: if i == 0 { SegmentKind::Identifier } else { SegmentKind::Property },
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                })
                .collect(),
        }
    }

    fn make_file_ctx_with_imports(libs: &[&str]) -> FileContext {
        FileContext {
            file_path: "src/api.py".to_string(),
            language: "python".to_string(),
            imports: libs
                .iter()
                .map(|lib| ImportEntry {
                    imported_name: lib.to_string(),
                    module_path: Some(lib.to_string()),
                    alias: None,
                    is_wildcard: false,
                })
                .collect(),
            file_namespace: None,
        }
    }

    #[test]
    fn fastapi_get_decorator_emits_consumer() {
        use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
        match detect_python_route_decorator_emission("app.get", Some("/users/{id}")).unwrap() {
            FlowEmission::NamedChannel { kind, role, name, method, .. } => {
                assert_eq!(kind, NamedChannelKind::HttpCall);
                assert_eq!(role, ChannelRole::Consumer);
                assert_eq!(name, "/users/{}");
                assert_eq!(method, Some(HttpMethod::Get));
            }
            other => panic!("expected NamedChannel HttpCall, got {other:?}"),
        }
    }

    #[test]
    fn fastapi_router_post_emits_post() {
        use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
        match detect_python_route_decorator_emission("router.post", Some("/login")).unwrap() {
            FlowEmission::NamedChannel { method, name, .. } => {
                assert_eq!(method, Some(HttpMethod::Post));
                assert_eq!(name, "/login");
            }
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn flask_route_decorator_emits_any() {
        use crate::indexer::resolve::flow_emit::{FlowEmission, HttpMethod};
        match detect_python_route_decorator_emission("app.route", Some("/x")).unwrap() {
            FlowEmission::NamedChannel { method, .. } => {
                assert_eq!(method, Some(HttpMethod::Any));
            }
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn route_decorator_no_emit_for_unrelated_decorator() {
        assert!(detect_python_route_decorator_emission("dataclass", None).is_none());
        assert!(detect_python_route_decorator_emission("classmethod", None).is_none());
        assert!(detect_python_route_decorator_emission("app.get", None).is_none());
    }

    #[test]
    fn requests_get_emits_producer() {
        use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
        let chain = make_chain(&["requests", "get"]);
        let call_args = vec![CallArg::StringLit("/api/users".to_string())];
        let ctx = make_file_ctx_with_imports(&["requests"]);
        match detect_python_http_chain_emission(&chain, &call_args, &ctx).unwrap() {
            FlowEmission::NamedChannel { kind, role, name, method, .. } => {
                assert_eq!(kind, NamedChannelKind::HttpCall);
                assert_eq!(role, ChannelRole::Producer);
                assert_eq!(name, "/api/users");
                assert_eq!(method, Some(HttpMethod::Get));
            }
            other => panic!("expected NamedChannel HttpCall, got {other:?}"),
        }
    }

    #[test]
    fn httpx_async_client_get_emits_producer() {
        use crate::indexer::resolve::flow_emit::FlowEmission;
        let chain = make_chain(&["client", "get"]);
        let call_args = vec![CallArg::StringLit("/api/me".to_string())];
        let ctx = make_file_ctx_with_imports(&["httpx"]);
        match detect_python_http_chain_emission(&chain, &call_args, &ctx).unwrap() {
            FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/api/me"),
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn aiohttp_session_get_emits_producer() {
        use crate::indexer::resolve::flow_emit::FlowEmission;
        let chain = make_chain(&["session", "get"]);
        let call_args = vec![CallArg::StringLit("/api/data".to_string())];
        let ctx = make_file_ctx_with_imports(&["aiohttp"]);
        match detect_python_http_chain_emission(&chain, &call_args, &ctx).unwrap() {
            FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/api/data"),
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn http_producer_no_emit_without_library_import() {
        let chain = make_chain(&["obj", "get"]);
        let call_args = vec![CallArg::StringLit("key".to_string())];
        let ctx = make_file_ctx_with_imports(&["typing"]);
        assert!(detect_python_http_chain_emission(&chain, &call_args, &ctx).is_none());
    }

    #[test]
    fn sqlalchemy_entity_query_filter_emits_dbquery() {
        use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
        let chain = make_chain(&["User", "query", "filter"]);
        match detect_python_db_query_emission(&chain).unwrap() {
            FlowEmission::DbQuery { entity_name, operation } => {
                assert_eq!(entity_name, "py.User");
                assert_eq!(operation, DbQueryOp::Select);
            }
            other => panic!("expected DbQuery, got {other:?}"),
        }
    }

    #[test]
    fn django_entity_objects_create_emits_dbquery_insert() {
        use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
        let chain = make_chain(&["User", "objects", "create"]);
        match detect_python_db_query_emission(&chain).unwrap() {
            FlowEmission::DbQuery { entity_name, operation } => {
                assert_eq!(entity_name, "py.User");
                assert_eq!(operation, DbQueryOp::Insert);
            }
            _ => panic!("expected DbQuery"),
        }
    }

    #[test]
    fn django_entity_objects_filter_emits_dbquery_select() {
        use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
        let chain = make_chain(&["Poll", "objects", "filter"]);
        match detect_python_db_query_emission(&chain).unwrap() {
            FlowEmission::DbQuery { entity_name, operation } => {
                assert_eq!(entity_name, "py.Poll");
                assert_eq!(operation, DbQueryOp::Select);
            }
            _ => panic!("expected DbQuery"),
        }
    }

    #[test]
    fn db_query_no_emit_for_unrelated_chain() {
        let chain = make_chain(&["some", "thing", "method"]);
        assert!(detect_python_db_query_emission(&chain).is_none());
        let chain2 = make_chain(&["user", "objects", "all"]);
        assert!(detect_python_db_query_emission(&chain2).is_none());
    }

    // -----------------------------------------------------------------------
    // Goal 19 — extended detectors
    // -----------------------------------------------------------------------

    #[test]
    fn django_path_emits_consumer_http() {
        use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
        let args = vec![CallArg::StringLit("users/".to_string()), CallArg::Other];
        let ctx = make_file_ctx_with_imports(&["django"]);
        match detect_python_django_path_emission("path", &args, &ctx).unwrap() {
            FlowEmission::NamedChannel { kind, role, name, method, .. } => {
                assert_eq!(kind, NamedChannelKind::HttpCall);
                assert_eq!(role, ChannelRole::Consumer);
                assert_eq!(name, "/users");
                assert_eq!(method, Some(HttpMethod::Any));
            }
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn django_re_path_strips_anchors() {
        use crate::indexer::resolve::flow_emit::FlowEmission;
        let args = vec![CallArg::StringLit("^users/$".to_string())];
        let ctx = make_file_ctx_with_imports(&["django"]);
        match detect_python_django_path_emission("re_path", &args, &ctx).unwrap() {
            FlowEmission::NamedChannel { name, .. } => assert_eq!(name, "/users"),
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn django_path_requires_django_import() {
        let args = vec![CallArg::StringLit("users/".to_string())];
        let ctx = make_file_ctx_with_imports(&["typing"]);
        assert!(detect_python_django_path_emission("path", &args, &ctx).is_none());
    }

    #[test]
    fn channels_consumer_inheritance_emits_ws() {
        use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
        match detect_python_channels_consumer_inheritance("AsyncWebsocketConsumer").unwrap() {
            FlowEmission::NamedChannel { kind, role, .. } => {
                assert!(matches!(kind, NamedChannelKind::WebSocket));
                assert_eq!(role, ChannelRole::Consumer);
            }
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn channels_json_consumer_also_recognised() {
        assert!(detect_python_channels_consumer_inheritance("JsonWebsocketConsumer").is_some());
        assert!(detect_python_channels_consumer_inheritance("AsyncJsonWebsocketConsumer").is_some());
    }

    #[test]
    fn channels_rejects_non_consumer_base() {
        assert!(detect_python_channels_consumer_inheritance("View").is_none());
        assert!(detect_python_channels_consumer_inheritance("models.Model").is_none());
    }

    #[test]
    fn channels_path_emits_ws_when_channels_imported() {
        use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
        let args = vec![CallArg::StringLit("ws/chat/".to_string())];
        let ctx = make_file_ctx_with_imports(&["channels"]);
        match detect_python_channels_path_emission("path", &args, &ctx).unwrap() {
            FlowEmission::NamedChannel { kind, role, name, .. } => {
                assert!(matches!(kind, NamedChannelKind::WebSocket));
                assert_eq!(role, ChannelRole::Consumer);
                assert_eq!(name, "/ws/chat");
            }
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn channels_path_requires_channels_import() {
        let args = vec![CallArg::StringLit("ws/chat/".to_string())];
        let ctx = make_file_ctx_with_imports(&["django"]);
        assert!(detect_python_channels_path_emission("path", &args, &ctx).is_none());
    }

    #[test]
    fn strawberry_field_emits_graphql_consumer() {
        use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
        match detect_python_graphql_decorator_emission("strawberry.field", "users").unwrap() {
            FlowEmission::NamedChannel { kind, role, name, .. } => {
                assert!(matches!(kind, NamedChannelKind::GraphQLOp));
                assert_eq!(role, ChannelRole::Consumer);
                assert_eq!(name, "field:users");
            }
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn graphene_mutation_recognised() {
        assert!(
            detect_python_graphql_decorator_emission("graphene.mutation", "createUser").is_some()
        );
    }

    #[test]
    fn graphql_rejects_non_op_decorator() {
        // `@strawberry.type` marks a schema type, not a callable op — skip
        // until DbEntity-style pairing is wired for GraphQL types.
        assert!(detect_python_graphql_decorator_emission("strawberry.type", "User").is_none());
    }

    #[test]
    fn graphql_rejects_unrelated_decorator() {
        assert!(detect_python_graphql_decorator_emission("functools.cache", "f").is_none());
    }

    #[test]
    fn urllib_urlopen_emits_producer() {
        use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, HttpMethod, NamedChannelKind};
        let chain = make_chain(&["urllib", "request", "urlopen"]);
        let args = vec![CallArg::StringLit("https://api.example.com/x".to_string())];
        let ctx = make_file_ctx_with_imports(&["urllib"]);
        match detect_python_http_chain_emission(&chain, &args, &ctx).unwrap() {
            FlowEmission::NamedChannel { kind, role, method, .. } => {
                assert_eq!(kind, NamedChannelKind::HttpCall);
                assert_eq!(role, ChannelRole::Producer);
                assert_eq!(method, Some(HttpMethod::Any));
            }
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn sqlalchemy_select_call_emits_db_query() {
        use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
        let args = vec![CallArg::Ident("User".to_string())];
        let ctx = make_file_ctx_with_imports(&["sqlalchemy"]);
        match detect_python_sqlalchemy_select_call("select", &args, &ctx).unwrap() {
            FlowEmission::DbQuery { entity_name, operation } => {
                assert_eq!(entity_name, "py.User");
                assert_eq!(operation, DbQueryOp::Select);
            }
            _ => panic!("expected DbQuery"),
        }
    }

    #[test]
    fn sqlalchemy_select_requires_sqlalchemy_import() {
        let args = vec![CallArg::Ident("User".to_string())];
        let ctx = make_file_ctx_with_imports(&["typing"]);
        assert!(detect_python_sqlalchemy_select_call("select", &args, &ctx).is_none());
    }

    #[test]
    fn cursor_execute_select_emits_db_query() {
        use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
        let chain = make_chain(&["cursor", "execute"]);
        let args = vec![CallArg::StringLit("SELECT id FROM users WHERE active = 1".to_string())];
        match detect_python_cursor_execute_emission(&chain, &args).unwrap() {
            FlowEmission::DbQuery { entity_name, operation } => {
                assert_eq!(entity_name, "py.users");
                assert_eq!(operation, DbQueryOp::Select);
            }
            _ => panic!("expected DbQuery"),
        }
    }

    #[test]
    fn cursor_execute_insert_emits_op() {
        use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
        let chain = make_chain(&["cursor", "execute"]);
        let args = vec![CallArg::StringLit("INSERT INTO items (a) VALUES (1)".to_string())];
        match detect_python_cursor_execute_emission(&chain, &args).unwrap() {
            FlowEmission::DbQuery { entity_name, operation } => {
                assert_eq!(entity_name, "py.items");
                assert_eq!(operation, DbQueryOp::Insert);
            }
            _ => panic!("expected DbQuery"),
        }
    }

    #[test]
    fn grpc_stub_method_emits_rpc_call() {
        use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
        let chain = make_chain(&["UserServiceStub", "GetUser"]);
        match detect_python_grpc_stub_emission(&chain).unwrap() {
            FlowEmission::NamedChannel { kind, role, name, .. } => {
                assert!(matches!(kind, NamedChannelKind::RpcCall));
                assert_eq!(role, ChannelRole::Producer);
                assert_eq!(name, "UserService.GetUser");
            }
            _ => panic!("expected NamedChannel"),
        }
    }

    #[test]
    fn grpc_stub_rejects_non_stub_root() {
        let chain = make_chain(&["UserService", "GetUser"]);
        assert!(detect_python_grpc_stub_emission(&chain).is_none());
    }
}
