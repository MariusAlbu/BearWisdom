// Dart language hooks. Absorbed from the deleted `dart/resolve.rs`.

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct DartHooks;

pub(crate) fn detect_dart_shelf_route(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if s.starts_with('/') => Some(s.as_str()),
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Consumer,
        method: Some(method),
        streaming: None,
    })
}

pub(crate) fn detect_dart_http_chain(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if chain.segments.len() < 2 {
        return None;
    }
    let leaf = chain.segments.last()?.name.as_str();
    let method = match leaf {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        _ => return None,
    };
    let root = chain.segments[0].name.as_str();
    if !matches!(root, "dio" | "http" | "client" | "_client" | "Dio") {
        return None;
    }
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Producer,
        method: Some(method),
        streaming: None,
    })
}

pub(crate) fn detect_dart_drift_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if chain.segments.len() < 2 {
        return None;
    }
    let root = chain.segments[0].name.as_str();
    let leaf = chain.segments.last()?.name.as_str();
    let op = match root {
        "select" => DbQueryOp::Select,
        "insert" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    if !matches!(
        leaf,
        "get" | "getSingle" | "watch" | "write" | "insert" | "go" | "go!" | "do"
    ) {
        return None;
    }
    Some(FlowEmission::DbQuery {
        entity_name: "dart.*".to_string(),
        operation: op,
    })
}

pub(crate) fn detect_dart_grpc_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    if chain.segments.len() < 2 {
        return None;
    }
    let root = chain.segments[0].name.as_str();
    if !root.ends_with("Client") || root == "Client" {
        return None;
    }
    let leaf = chain.segments.last()?.name.as_str();
    if matches!(leaf, "shutdown" | "terminate") {
        return None;
    }
    let service = root.strip_suffix("Client").unwrap_or(root);
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!("{}.{}", service, leaf),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
    })
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    if let Some(chain) = r.chain.as_ref() {
        if let Some(em) = detect_dart_http_chain(chain, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_dart_drift_emission(chain) {
            return vec![em];
        }
        if let Some(em) = detect_dart_grpc_emission(chain) {
            return vec![em];
        }
    }
    if let Some(em) = detect_dart_shelf_route(r.target_name.as_str(), &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

/// Classify a Dart import URI to its external namespace, or `None` when the
/// URI names project-local code. `dart:` URIs map to `dart.stdlib`;
/// `package:<pkg>/...` URIs map to `<pkg>` when the package is a manifest
/// dependency or matches the known-external predicate. A `package:<self>/...`
/// URI — where `<self>` is the project's own pubspec `name:` — is project-local
/// (the idiomatic intra-package absolute import); it returns `None` so the
/// import binds under `lib/` through the module-resolution plumbing instead of
/// being branded external.
pub(crate) fn classify_dart_import_uri(
    uri: &str,
    file_package_id: Option<i64>,
    project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    if let Some(pkg_path) = uri.strip_prefix("package:") {
        let pkg_name = pkg_path.split('/').next().unwrap_or(pkg_path);
        if is_own_package(pkg_name, file_package_id, project_ctx) {
            return None;
        }
    }
    if predicates::is_external_dart_import(uri) {
        let ns = if uri.starts_with("dart:") {
            "dart.stdlib"
        } else if let Some(pkg_path) = uri.strip_prefix("package:") {
            pkg_path.split('/').next().unwrap_or(pkg_path)
        } else {
            uri
        };
        return Some(ns.to_string());
    }
    if let Some(pkg_path) = uri.strip_prefix("package:") {
        let pkg_name = pkg_path.split('/').next().unwrap_or(pkg_path);
        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx
                .manifests_for(file_package_id)
                .get(&ManifestKind::Pubspec)
            {
                if manifest.dependencies.contains(pkg_name) {
                    return Some(pkg_name.to_string());
                }
            }
        }
    }
    None
}

/// Whether `pkg_name` names the project's own pubspec package. The own name is
/// the pubspec `name:`, folded into the package-visible `ManifestData`'s
/// `package_names` (the same signal `DartModuleResolver` consumes to map a
/// `package:<self>/...` URI to `lib/`). In a melos workspace `package_names`
/// carries every member's name, so a sibling-package `package:` URI is also
/// recognized as project-local. Returns false when no pubspec is visible.
fn is_own_package(
    pkg_name: &str,
    file_package_id: Option<i64>,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    project_ctx
        .and_then(|ctx| {
            ctx.manifests_for(file_package_id)
                .get(&ManifestKind::Pubspec)
        })
        .is_some_and(|manifest| manifest.package_names.iter().any(|n| n == pkg_name))
}

/// The bare package name a Dart `package:<pkg>/<rest>` URI imports from, or
/// `None` for relative / `dart:` / unparseable URIs. `package:foo/bar/baz.dart`
/// → `foo`.
fn package_uri_root(uri: &str) -> Option<&str> {
    let rest = uri.strip_prefix("package:")?;
    let root = rest.split('/').next().unwrap_or(rest);
    (!root.is_empty()).then_some(root)
}

/// Bind a bare type/instantiate/inherit/call ref to a symbol declared in a
/// sibling workspace package the file imports via `package:`.
///
/// Dart imports whole libraries — a `package:<pkg>/...` directive brings every
/// public name of `<pkg>` into scope with no per-symbol binding, so a bare type
/// name carries no module and the generic import-anchored strategies have
/// nothing to key on. This scans the file's `package:` imports, maps each to a
/// workspace member by its pubspec-declared name, and looks up the target name
/// (kind-compatible) among that member's symbols. Scoping the search to the
/// imported member resolves the same-name-different-kind case: a class in the
/// imported package wins over a same-named property declared elsewhere.
///
/// Imports that resolve to the file's own package are skipped — intra-package
/// references resolve on the generic path before this runs, and a same-package
/// `package:<self>/...` import must not shadow that.
fn resolve_via_dart_package_import(
    ref_ctx: &RefContext<'_>,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let target = ref_ctx.extracted_ref.target_name.as_str();
    if target.is_empty() || target.contains('.') {
        return None;
    }
    let edge_kind = ref_ctx.extracted_ref.kind;
    if !matches!(
        edge_kind,
        EdgeKind::TypeRef | EdgeKind::Instantiates | EdgeKind::Inherits | EdgeKind::Implements
    ) {
        return None;
    }
    let own_pkg = ref_ctx.file_package_id;
    for import in &file_ctx.imports {
        let uri = import.module_path.as_deref().unwrap_or("");
        let Some(pkg_name) = package_uri_root(uri) else {
            continue;
        };
        let Some(pkg_id) = lookup.workspace_package_id(pkg_name) else {
            continue;
        };
        if own_pkg == Some(pkg_id) {
            continue;
        }
        for sym in lookup.symbols_in_package(pkg_id) {
            if sym.name == target && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: RESOLVED_CONFIDENCE,
                    strategy: "dart_workspace_package_import",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    None
}

impl LanguageEngineHooks for DartHooks {
    fn resolve_bare_post(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        resolve_via_dart_package_import(ref_ctx, file_ctx, lookup)
    }

    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let uri = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            return classify_dart_import_uri(uri, ref_ctx.file_package_id, project_ctx);
        }
        // A qualified reference `i0.Value` carries its library prefix on
        // `namespace_segments[0]` and the prefix's import URI on `module` (the
        // extractor splits the prefix from the name and routes it through the
        // import). Classify off that URI directly — when it names an external
        // library the ref is external.
        if !ref_ctx.extracted_ref.namespace_segments.is_empty() {
            if let Some(uri) = ref_ctx.extracted_ref.module.as_deref() {
                if let Some(ns) =
                    classify_dart_import_uri(uri, ref_ctx.file_package_id, project_ctx)
                {
                    return Some(ns);
                }
            }
        }
        let simple = target.split('.').next().unwrap_or(target);
        for import in &file_ctx.imports {
            let uri = import.module_path.as_deref().unwrap_or("");
            if uri.is_empty() {
                continue;
            }
            let pkg_name_from_uri = if uri.starts_with("package:") {
                uri.strip_prefix("package:")
                    .unwrap_or(uri)
                    .split('/')
                    .next()
                    .unwrap_or(uri)
            } else {
                ""
            };
            // A `package:<self>/...` import is project-local; it must not brand
            // any ref external through this import, regardless of alias.
            if !pkg_name_from_uri.is_empty()
                && is_own_package(pkg_name_from_uri, ref_ctx.file_package_id, project_ctx)
            {
                continue;
            }
            let is_manifest_external = !pkg_name_from_uri.is_empty()
                && project_ctx
                    .and_then(|ctx| {
                        ctx.manifests_for(ref_ctx.file_package_id)
                            .get(&ManifestKind::Pubspec)
                    })
                    .is_some_and(|m| m.dependencies.contains(pkg_name_from_uri));
            if let Some(alias) = &import.alias {
                if alias == simple
                    && (is_manifest_external || predicates::is_external_dart_import(uri))
                {
                    if uri.starts_with("package:") {
                        return Some(pkg_name_from_uri.to_string());
                    }
                    return Some(uri.to_string());
                }
            }
            if import.alias.is_none() {
                if is_manifest_external {
                    return Some(pkg_name_from_uri.to_string());
                }
                if predicates::is_external_dart_import(uri) {
                    if uri.starts_with("package:") {
                        return Some(pkg_name_from_uri.to_string());
                    }
                    return Some("dart.stdlib".to_string());
                }
            }
        }
        None
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        _lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let uri = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != uri {
                Some(r.target_name.clone())
            } else {
                None
            };
            imports.push(ImportEntry {
                imported_name: uri.to_string(),
                module_path: Some(uri.to_string()),
                alias,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "dart".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static DART_HOOKS: DartHooks = DartHooks;
