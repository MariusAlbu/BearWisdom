// =============================================================================
// languages/python/hooks.rs — PythonHooks impl plus the concrete
// PythonResolver (chain-aware via PythonChecker, module-qualified
// Inherits/TypeRef like `class Foo(models.TextChoices)`, scope-chain walk
// with `self.` stripping, same-file lookup, fully-qualified-name with
// module-alias resolution, from-import resolution covering exact match +
// module-prefix-by-name with __init__.py re-exports) plus the detect_flow
// detectors (route decorators / Django Channels / GraphQL / path() /
// sqlalchemy select / HTTP chain / DB query / cursor.execute / gRPC stub /
// mailer / bgjob / redis) and file-context builder.
// =============================================================================

use super::externals;
use super::flow_detectors::{
    detect_python_bgjob_emission, detect_python_channels_consumer_inheritance,
    detect_python_channels_path_emission, detect_python_cursor_execute_emission,
    detect_python_db_query_emission, detect_python_django_path_emission,
    detect_python_graphql_decorator_emission, detect_python_grpc_stub_emission,
    detect_python_http_chain_emission, detect_python_mailer_emission, detect_python_redis_lookup,
    detect_python_route_decorator_emission, detect_python_sqlalchemy_select_call,
};
#[cfg(test)]
use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct PythonResolver;

/// Python's chain case-space, expressed as a `ChainConfig` and exercised
/// through `resolve_via_chain` by the differential tests in
/// `type_checker/chain_tests`. The production chain path for Python is the
/// profile-driven engine `ChainWalker` (see `type_checker/engine.rs`); this
/// config pins the same bare three-phase shape the walker must reproduce:
/// `has_self_ref` for `self.`-rooted chains, an `Identifier` root resolved
/// through `local_type` → static-type-name → enclosing-field type → declared
/// type, then field/return/members_of progression. No generics
/// (`use_generics: false`), no namespace-qualified lookups, no external-qname
/// promotion, no construction roots, no inheritance climb, and no static `::`
/// roots. `expand_aliases` is on: a value typed as a PEP 613 / `type X = ...`
/// alias name walks to the alias's target before member lookup.
#[cfg(test)]
pub(crate) static PYTHON_CHAIN_CONFIG: crate::type_checker::chain::ChainConfig =
    crate::type_checker::chain::ChainConfig {
        strategy_prefix: "python",
        normalize_type: crate::type_checker::chain::identity_normalize,
        has_self_ref: true,
        enclosing_type_kinds: &["class", "struct", "interface"],
        static_type_kinds: &["class", "struct", "interface", "enum", "type_alias"],
        use_generics: false,
        namespace_lookup: crate::type_checker::chain::NamespaceLookup::None,
        kind_compatible: predicates::kind_compatible,
        extensions: crate::type_checker::chain::ChainExtensions {
            expand_aliases: true,
            walk_inheritance: false,
            promote_external_qname: false,
            root_construction: false,
            extension_method_fallback: false,
            root_fallback: None,
            root_type_access: false,
            qualify_via_imports: false,
        },
    };

impl PythonResolver {
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
}

pub(crate) fn detect_flow_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;

    if r.kind == EdgeKind::TypeRef {
        if let Some(emission) =
            detect_python_route_decorator_emission(r.target_name.as_str(), r.module.as_deref())
        {
            return vec![emission];
        }
        // Django Channels: `class XConsumer(AsyncWebsocketConsumer)`.
        if let Some(emission) = detect_python_channels_consumer_inheritance(r.target_name.as_str())
        {
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
        if let Some(emission) =
            detect_python_channels_path_emission(r.target_name.as_str(), &r.call_args, file_ctx)
        {
            return vec![emission];
        }
        if let Some(emission) =
            detect_python_django_path_emission(r.target_name.as_str(), &r.call_args, file_ctx)
        {
            return vec![emission];
        }
        if let Some(emission) =
            detect_python_sqlalchemy_select_call(r.target_name.as_str(), &r.call_args, file_ctx)
        {
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
    let Some(chain) = r.chain.as_ref() else {
        return Vec::new();
    };
    let Some(root_seg) = chain.segments.first() else {
        return Vec::new();
    };
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
        is_call: false,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }];
    new_segments.extend(chain.segments.iter().skip(1).cloned());
    let rewritten = crate::types::MemberChain {
        segments: new_segments,
    };
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
}

pub static PYTHON_HOOKS: PythonHooks = PythonHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod hooks_tests;
