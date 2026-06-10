// =============================================================================
// languages/elixir/hooks.rs — ElixirHooks impl: file-context construction
// (the alias→module-qname import table the generic ladder consumes),
// 6 flow detectors (Ecto Repo ops on `.Repo`-suffix modules,
// HTTPoison/Tesla/Req/Finch/Mojito chains, grpc-elixir generated stubs ending
// in `.Stub`, Oban bgjob, Phoenix.Channel / Phoenix.LiveView WebSocket
// Consumer from `use` macro, Bamboo/Swoosh mailer deliver_*), plus
// infer_external_inner with mix.exs dep matching (CamelCase root ↔
// snake_case dep atom, plus first-segment prefix), the `Routes`
// Phoenix-convention universal alias rule, import-wildcard fallback for
// `import Bamboo.Test`-style injection, and use-injection inference via
// `lookup.by_qualified_name(module.target)` confirmation against the
// externals symbol set. Bare-alias binding (`alias MyApp.Foo` → `Foo`) is
// handled by the generic ladder's `resolve_via_alias_module_qname` strategy,
// gated by `ELIXIR_PROFILE.alias_module_qname`.
// =============================================================================

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub(crate) fn detect_elixir_ecto_emission(
    module: &str,
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let mod_last = module.rsplit('.').next().unwrap_or(module);
    if !matches!(mod_last, "Repo" | "Repos" | "TenantRepo") && !module.ends_with(".Repo") {
        return None;
    }
    let op = match target {
        "get" | "get!" | "get_by" | "get_by!" | "one" | "one!" | "all" | "stream" | "exists?"
        | "aggregate" | "preload" => DbQueryOp::Select,
        "insert" | "insert!" | "insert_all" | "insert_or_update" | "insert_or_update!" => {
            DbQueryOp::Insert
        }
        "update" | "update!" | "update_all" => DbQueryOp::Update,
        "delete" | "delete!" | "delete_all" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "ex.*".to_string(),
        operation: op,
    })
}

pub(crate) fn detect_elixir_http_emission(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let mod_last = module.rsplit('.').next().unwrap_or(module);
    if !matches!(mod_last, "HTTPoison" | "Tesla" | "Req" | "Finch" | "Mojito") {
        return None;
    }
    let method = match target {
        "get" | "get!" => HttpMethod::Get,
        "post" | "post!" => HttpMethod::Post,
        "put" | "put!" => HttpMethod::Put,
        "patch" | "patch!" => HttpMethod::Patch,
        "delete" | "delete!" => HttpMethod::Delete,
        "head" | "head!" => HttpMethod::Head,
        "request" | "request!" => HttpMethod::Any,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    })?;
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Producer,
        method: Some(method),
        streaming: None,
    })
}

pub(crate) fn detect_elixir_grpc_emission(
    module: &str,
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    // grpc-elixir generated stubs: `MyApp.UserService.Stub.get_user(...)`.
    let mod_last = module.rsplit('.').next().unwrap_or(module);
    if mod_last != "Stub" {
        return None;
    }
    if matches!(target, "start_link" | "init" | "stop") {
        return None;
    }
    let parts: Vec<&str> = module.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let service = parts[parts.len() - 2];
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!("{}.{}", service, target),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(target)),
    })
}

pub(crate) fn detect_elixir_oban_emission(
    module: &str,
    target: &str,
    _call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let mod_last = module.rsplit('.').next().unwrap_or(module);
    if mod_last != "Oban" || !matches!(target, "insert" | "insert_all") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: "oban.job".to_string(),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

// `use Phoenix.Channel` / `use Phoenix.LiveView` declares the surrounding
// module as a WebSocket Consumer. The extractor emits these as Imports
// refs; the module path lives on either `target_name` or `module`.
pub(crate) fn detect_elixir_phoenix_channel_use(
    target_name: &str,
    module: Option<&str>,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let canonical = module.unwrap_or(target_name);
    if !matches!(canonical, "Phoenix.Channel" | "Phoenix.LiveView")
        && !canonical.ends_with(".Channel")
        && !canonical.ends_with(".LiveView")
    {
        return None;
    }
    if !canonical.starts_with("Phoenix") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: format!("ex.{}", canonical),
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    })
}

// Bamboo `MyMailer.deliver_now(email)` and Swoosh `MyMailer.deliver(email)`.
// Module name is project-specific (`MyApp.Mailer`); leaf op is stable.
pub(crate) fn detect_elixir_mailer_emission(
    module: &str,
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    if !matches!(
        target,
        "deliver" | "deliver_now" | "deliver_later" | "deliver_now!" | "deliver_later!"
    ) {
        return None;
    }
    let mod_last = module.rsplit('.').next().unwrap_or(module);
    if !mod_last.ends_with("Mailer") && mod_last != "Bamboo" && mod_last != "Swoosh" {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("ex.{}", mod_last),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

// CamelCase Elixir module root ↔ snake_case mix.exs dep atom.
// Direct lowercase match ("Phoenix" → "phoenix") plus first-segment prefix
// ("ecto_sql" matches "Ecto").
fn is_mix_dep_match(module_root: &str, deps: &std::collections::HashSet<String>) -> bool {
    let root_lower = module_root.to_lowercase();
    for dep in deps {
        if dep == &root_lower {
            return true;
        }
        if let Some(prefix) = dep.split('_').next() {
            if prefix == root_lower {
                return true;
            }
        }
    }
    false
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        let root = module.split('.').next().unwrap_or(module);

        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Mix)
            {
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

    // ref.module set (`module="Ecto.Changeset"` on a type_ref to "Changeset").
    if let Some(module) = &ref_ctx.extracted_ref.module {
        let root = module.split('.').next().unwrap_or(module);
        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Mix)
            {
                if is_mix_dep_match(root, &manifest.dependencies) {
                    return Some(root.to_string());
                }
            }
        }
        if predicates::is_external_elixir_module(module) {
            return Some(root.to_string());
        }
    }

    // Target matches a known-external alias in this file.
    for import in &file_ctx.imports {
        if import.imported_name != *target {
            continue;
        }
        let module = import.module_path.as_deref().unwrap_or("");
        let root = module.split('.').next().unwrap_or(module);

        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Mix)
            {
                if is_mix_dep_match(root, &manifest.dependencies) {
                    return Some(root.to_string());
                }
            }
        }

        if predicates::is_external_elixir_module(module) {
            return Some(root.to_string());
        }
    }

    if target.contains('.') {
        let root = target.split('.').next().unwrap_or(target);

        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Mix)
            {
                if is_mix_dep_match(root, &manifest.dependencies) {
                    return Some(root.to_string());
                }
            }
        }

        if predicates::is_external_elixir_module(root) {
            return Some(root.to_string());
        }
    } else {
        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Mix)
            {
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
    // symbol. Resolved through four injection paths:
    //   1. Direct alias: `alias MyApp.Router.Helpers, as: Routes`.
    //   2. ConnCase test-case injection (handled elsewhere).
    //   3. Web wrapper: `use MyAppWeb, :controller` quote-block injection.
    //   4. Project-internal macro: `use MyApp.SomeResource` whose
    //      `defmacro __using__` quote block aliases Routes invisibly.
    if target == "Routes" {
        for import in &file_ctx.imports {
            let mp = import.module_path.as_deref().unwrap_or("");
            if mp.ends_with("Router.Helpers") {
                return Some("Phoenix".to_string());
            }
            let last = mp.split('.').last().unwrap_or(mp);
            if last.ends_with("Web") && !last.is_empty() {
                return Some("Phoenix".to_string());
            }
        }
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
                .map(|m| {
                    !m.is_empty() && m.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                })
                .unwrap_or(false)
        });
        if phoenix_in_mix && has_internal_use {
            return Some("Phoenix".to_string());
        }
    }

    // Bare-import wildcard fallback. `import Bamboo.Test` brings every
    // public function into scope — assert_email_delivered_with, etc.
    // Heuristic: bare unresolved Calls with function-name shape (lowercase
    // first letter) AND an Imports ref where imported_name == last_segment
    // (no `as:` rename) AND that module is in Mix deps → attribute to dep.
    if ref_ctx.extracted_ref.kind == EdgeKind::Calls
        && ref_ctx.extracted_ref.module.is_none()
        && !target.contains('.')
        && target
            .chars()
            .next()
            .map(|c| c.is_lowercase() || c == '_')
            .unwrap_or(false)
    {
        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Mix)
            {
                for import in &file_ctx.imports {
                    let Some(module_path) = import.module_path.as_deref() else {
                        continue;
                    };
                    let last_segment = module_path.split('.').last().unwrap_or(module_path);
                    // Skip alias-with-as: imported_name differs from
                    // last_segment when `as: X` was used.
                    if import.imported_name != last_segment {
                        continue;
                    }
                    let root = module_path.split('.').next().unwrap_or(module_path);
                    if is_mix_dep_match(root, &manifest.dependencies) {
                        return Some(root.to_string());
                    }
                }
            }
        }
    }

    None
}

pub(crate) fn infer_external_inner_with_lookup(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    if let Some(ns) = infer_external_inner(file_ctx, ref_ctx, project_ctx) {
        return Some(ns);
    }

    // Use-injection inference: bare Calls only.
    if ref_ctx.extracted_ref.kind != EdgeKind::Calls {
        return None;
    }
    let target = &ref_ctx.extracted_ref.target_name;
    if target.is_empty() {
        return None;
    }

    for import in &file_ctx.imports {
        let module = import.module_path.as_deref().unwrap_or("");
        if module.is_empty() {
            continue;
        }
        let root = module.split('.').next().unwrap_or(module);
        let is_external_module = if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx
                .manifests_for(ref_ctx.file_package_id)
                .get(&ManifestKind::Mix)
            {
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

        let member_qname = format!("{module}.{target}");
        if lookup.by_qualified_name(&member_qname).is_some() {
            return Some(root.to_string());
        }
    }

    None
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;
    // Phoenix Channel macro: `use Phoenix.Channel` lands as Imports.
    if r.kind == EdgeKind::Imports {
        if let Some(em) =
            detect_elixir_phoenix_channel_use(r.target_name.as_str(), r.module.as_deref())
        {
            return vec![em];
        }
        return Vec::new();
    }
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let module = r.module.as_deref().unwrap_or("");
    let target = r.target_name.as_str();

    if let Some(em) = detect_elixir_ecto_emission(module, target) {
        return vec![em];
    }
    if let Some(em) = detect_elixir_http_emission(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_elixir_grpc_emission(module, target) {
        return vec![em];
    }
    if let Some(em) = detect_elixir_oban_emission(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_elixir_mailer_emission(module, target) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    let file_namespace = file.symbols.iter().find_map(|sym| {
        if sym.kind == crate::types::SymbolKind::Module
            || sym.kind == crate::types::SymbolKind::Namespace
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

        // Local binding name:
        //   - `module` set → target_name is the local alias.
        //   - no `module` → target_name is the module itself; use last segment.
        let imported_name = if r.module.is_some() {
            r.target_name.clone()
        } else {
            full_module
                .split('.')
                .last()
                .unwrap_or(&r.target_name)
                .to_string()
        };

        // `as:` was used when local name differs from last segment.
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

pub struct ElixirHooks;

impl LanguageEngineHooks for ElixirHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner_with_lookup(file_ctx, ref_ctx, project_ctx, lookup)
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }
}

pub static ELIXIR_HOOKS: ElixirHooks = ElixirHooks;
