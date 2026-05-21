// =============================================================================
// languages/python/hooks.rs — PythonHooks impl plus the concrete
// PythonResolver (chain-aware via PythonChecker, synthetic-global lookup for
// cpython stdlib symbols, module-qualified Inherits/TypeRef like
// `class Foo(models.TextChoices)`, scope-chain walk with `self.` stripping,
// same-file lookup, fully-qualified-name with module-alias resolution,
// from-import resolution covering exact match + module-prefix-by-name with
// __init__.py re-exports, file-extension-gated bare-name fallback for
// unittest mixins through Django TestCase deep hierarchies) plus the
// detect_flow detectors (route decorators / Django Channels / GraphQL /
// path() / sqlalchemy select / HTTP chain / DB query / cursor.execute /
// gRPC stub / mailer / bgjob / redis) and file-context builder.
// =============================================================================

use super::externals;
use super::flow_detectors::{
    detect_python_bgjob_emission, detect_python_channels_consumer_inheritance,
    detect_python_channels_path_emission, detect_python_cursor_execute_emission,
    detect_python_db_query_emission, detect_python_django_path_emission,
    detect_python_graphql_decorator_emission, detect_python_grpc_stub_emission,
    detect_python_http_chain_emission, detect_python_mailer_emission,
    detect_python_redis_lookup, detect_python_route_decorator_emission,
    detect_python_sqlalchemy_select_call,
};
use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    intern_yield_type, ChainMiss, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::chain::simple_yield_type;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, MemberChain, ParsedFile, SegmentKind};
use tracing::debug;

pub struct PythonResolver;

impl PythonResolver {
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }

    pub(crate) fn resolve(
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

        // Bare-name walker lookup. cpython_stdlib emits real symbols for
        // `print`, `len`, `dict`, exception types, str/list/dict methods,
        // etc. under `ext:cpython-stdlib:`. Skip when the ref has a chain
        // — the chain walker's receiver-type context is more precise.
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

        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = walk_python_chain(chain_val, edge_kind, ref_ctx, lookup) {
                return Some(res);
            }
        }

        // If the ref carries a module path:
        //
        // (A) Import-statement refs (no chain): the module is the import source.
        //     If we can't resolve them here, return None.
        //
        // (B) Call refs with a module set by the extractor post-pass
        //     (`Person.objects.filter()` → module="posthog.models"): use
        //     the module to locate the target before falling through.
        if let Some(module) = &ref_ctx.extracted_ref.module {
            if predicates::is_relative_import(module) {
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
                // Module-qualified ref. Two sub-cases share the same lookup:
                //   (B1) Chain-bearing call — `Person.objects.filter()` with
                //        the extractor's post-pass attaching the absolute
                //        module path.
                //   (B2) Module-qualified Inherits / TypeRef without a chain
                //        — `class Foo(models.TextChoices)`,
                //        `field: models.CharField`. The ref has the module
                //        attached but no member-chain because there's no
                //        further dispatch beyond the type access.
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
                // module path. Handles `models.TextChoices` living at qname
                // `TextChoices` in `django/db/models/enums.py` rather than
                // at `django.db.models.TextChoices`.
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
                // Resolve the module against the file's import map.
                // `class Foo(models.TextChoices)` where the file has
                // `from django.db import models` — walk `django/db/` for
                // a `TextChoices` symbol.
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
            if ref_ctx.extracted_ref.chain.is_none() {
                return None;
            }
            // Case (B) falls through to scope chain walk below.
        }

        let effective_target = target.strip_prefix("self.").unwrap_or(target);

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

        // Fully qualified name (dotted target like "models.User").
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

                    // Search by name within that module, including
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

        // Import-based resolution for simple names.
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

            // Search by simple name scoped to the module. Two checks:
            // qualified name prefix (when the extractor embeds the module
            // path) OR file path under the module directory (handles
            // __init__.py re-exports where Person lives in
            // posthog/models/person.py but is imported as
            // `from posthog.models import Person`).
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

        // Bare-name fallback for unittest-style assertion / mixin calls.
        // The chain walker can't follow `self.assertEqual` through
        // Django's deep TestCase hierarchy without inheritance type-flow,
        // so chain refs that resolve to a leaf method on `self` fall
        // through here. TypeRef accepts `method` here because
        // context-manager patterns like `with self.assertLogs(): …` are
        // emitted as TypeRef by the Python extractor.
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
}

/// Python chain walker.
pub(crate) fn walk_python_chain(
    chain_ref: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain_ref.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: Determine the root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            if let Some(local_type) = lookup.local_type(name) {
                Some(local_type)
            } else {
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "class" | "struct" | "interface" | "enum" | "type_alias"
                    )
                });
                if is_type {
                    Some(name.clone())
                } else {
                    let mut found = None;
                    for scope in &ref_ctx.scope_chain {
                        let field_qname = format!("{scope}.{name}");
                        if let Some(type_name) = lookup.field_type_str(&field_qname) {
                            found = Some(type_name.to_string());
                            break;
                        }
                    }
                    found.or_else(|| segments[0].declared_type.clone())
                }
            }
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // Phase 2: Walk intermediate segments.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(next_type) = lookup.field_type_str(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }
        if let Some(next_type) = lookup.return_type_str(&member_qname) {
            current_type = next_type.to_string();
            continue;
        }

        let mut found = false;
        for sym in lookup.members_of(&current_type) {
            if sym.name != seg.name {
                continue;
            }
            if let Some(ft) = lookup.field_type_str(&sym.qualified_name) {
                current_type = ft.to_string();
                found = true;
                break;
            }
            if let Some(rt) = lookup.return_type_str(&sym.qualified_name) {
                current_type = rt.to_string();
                found = true;
                break;
            }
        }
        if found {
            continue;
        }

        lookup.record_chain_miss(ChainMiss {
            current_type: current_type.clone(),
            target_name: seg.name.clone(),
        });
        return None;
    }

    // Phase 3: Final segment.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if predicates::kind_compatible(edge_kind, &sym.kind) {
            debug!(
                strategy = "python_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "python_chain_resolution",
                resolved_yield_type: intern_yield_type(simple_yield_type(sym, lookup), lookup),
                flow_emit: None,
            });
        }
    }

    for sym in lookup.members_of(&current_type) {
        if sym.name == last.name && predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "python_chain_resolution",
                resolved_yield_type: intern_yield_type(simple_yield_type(sym, lookup), lookup),
                flow_emit: None,
            });
        }
    }

    lookup.record_chain_miss(ChainMiss {
        current_type: current_type.clone(),
        target_name: last.name.clone(),
    });
    None
}

/// Find the enclosing class name from the scope chain.
fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    scope_chain.last().cloned()
}

pub(crate) fn detect_flow_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    if r.kind == EdgeKind::TypeRef {
        if let Some(emission) = detect_python_route_decorator_emission(
            r.target_name.as_str(),
            r.module.as_deref(),
        ) {
            return vec![emission];
        }
        // Django Channels: `class XConsumer(AsyncWebsocketConsumer)`.
        if let Some(emission) = detect_python_channels_consumer_inheritance(
            r.target_name.as_str(),
        ) {
            return vec![emission];
        }
        // Strawberry / Graphene GraphQL decorators.
        if let Some(emission) = detect_python_graphql_decorator_emission(
            r.target_name.as_str(),
            ref_ctx.source_symbol.name.as_str(),
        ) {
            return vec![emission];
        }
        return Vec::new();
    }

    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }

    // Django `path("users/", views.list)` / `re_path(...)` route
    // declarations land as Calls refs with no chain.
    if r.chain.is_none() {
        // Channels routing first: when the file imports `channels`,
        // `path("ws/x", X.as_asgi())` is a WS route, not HTTP.
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

pub(crate) fn detect_flow_inner_with_lookup(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let direct = detect_flow_inner(file_ctx, ref_ctx);
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
    let type_name = match lookup.field_type_str(&var_qname) {
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

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    // Python extractor emits:
    //   `import os`               → ref { target_name: "os",  module: None,    kind: Imports }
    //   `from foo.bar import Baz` → ref { target_name: "Baz", module: "foo.bar" }
    //   `from . import something` → ref { target_name: "something", module: "." }
    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }

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

pub struct PythonHooks;

impl LanguageEngineHooks for PythonHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        externals::infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        detect_flow_inner_with_lookup(file_ctx, ref_ctx, lookup)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        PythonResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static PYTHON_HOOKS: PythonHooks = PythonHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod hooks_tests;
