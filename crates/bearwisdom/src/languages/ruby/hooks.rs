// =============================================================================
// languages/ruby/hooks.rs — RubyHooks impl plus the concrete RubyResolver
// (require/require_relative landing on indexed files via in_module_from,
// chain via RubyChecker, scope-chain walk, same-file, same-module,
// fully-qualified-name with `::` ↔ `.` normalization, external gem lookup
// gated by imported gem list) plus 5 flow detectors (Net::HTTP/Faraday/
// HTTParty/RestClient client calls, ActionCable consumer inheritance,
// Sidekiq / ActiveJob bgjob, ActionMailer deliver_*, ActiveRecord ORM ops)
// plus external classifier (Gemfile manifest deps + bare gem requires) plus
// build_file_context (require + require_relative + Gemfile transitive
// requires as wildcard imports).
// =============================================================================

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct RubyResolver;

/// Ruby's chain case-space, expressed as a `ChainConfig` and exercised through
/// `resolve_via_chain` by the differential tests in `type_checker/chain_tests`.
/// The production chain path for Ruby is the profile-driven engine `ChainWalker`
/// (see `type_checker/engine.rs`); this config pins the same bare three-phase
/// shape the walker must reproduce: `has_self_ref` for `self.`-rooted chains
/// resolving the enclosing class/module from the scope chain, an `Identifier`
/// root through `local_type` → static-type-name → enclosing-field → declared
/// type, then field/return/members_of progression. Ruby modules index as
/// `namespace` symbols, so both enclosing and static kinds admit `namespace`.
/// No generics, no namespace-qualified lookups, and no `ChainExtensions`.
#[cfg(test)]
pub(crate) static RUBY_CHAIN_CONFIG: crate::type_checker::chain::ChainConfig =
    crate::type_checker::chain::ChainConfig {
        strategy_prefix: "ruby",
        normalize_type: crate::type_checker::chain::identity_normalize,
        has_self_ref: true,
        enclosing_type_kinds: &["class", "namespace", "interface"],
        static_type_kinds: &["class", "namespace", "interface", "type_alias"],
        use_generics: false,
        namespace_lookup: crate::type_checker::chain::NamespaceLookup::None,
        kind_compatible: predicates::kind_compatible,
        extensions: crate::type_checker::chain::ChainExtensions::NONE,
    };

impl RubyResolver {
    pub(crate) fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        build_file_context_inner(file, project_ctx)
    }
}

// Net::HTTP / Faraday / HTTParty / RestClient client calls.
pub(crate) fn detect_ruby_http_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let is_http_lib = match root {
        "Net" => segs.get(1).map_or(false, |s| s.name == "HTTP"),
        "Faraday" | "HTTParty" | "RestClient" => true,
        _ => false,
    };
    if !is_http_lib {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let method = match leaf {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        "patch" => HttpMethod::Patch,
        "head" => HttpMethod::Head,
        _ => return None,
    };
    let url = match call_args.first()? {
        crate::types::CallArg::StringLit(s) => s.clone(),
        crate::types::CallArg::TemplateLit(s) => s.clone(),
        _ => return None,
    };
    if !ruby_url_looks_like_api(&url) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: normalise_ruby_url(&url),
        role: ChannelRole::Producer,
        method: Some(method),
        streaming: None,
    })
}

fn ruby_url_looks_like_api(s: &str) -> bool {
    if s.starts_with("http://") || s.starts_with("https://") {
        let after = s.find("://").map(|i| &s[i + 3..]).unwrap_or(s);
        let path = after.find('/').map(|i| &after[i..]).unwrap_or("");
        if path.is_empty() {
            return false;
        }
        return ruby_url_looks_like_api(path);
    }
    s.starts_with('/')
        || s.contains("/api/")
        || s.contains("/v1/")
        || s.contains("/v2/")
        || s.contains("/{")
}

fn normalise_ruby_url(raw: &str) -> String {
    let without_query = raw.split('?').next().unwrap_or(raw);
    let re_tmpl = regex::Regex::new(r"#\{[^}]+\}").expect("template regex");
    re_tmpl.replace_all(without_query, "{param}").into_owned()
}

// `class XChannel < ApplicationCable::Channel` — Consumer WebSocket.
pub(crate) fn detect_ruby_actioncable_emission(
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let last = target.rsplit("::").next().unwrap_or(target);
    if last != "Channel" {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::WebSocket,
        name: "rb.actioncable".to_string(),
        role: ChannelRole::Consumer,
        method: None,
        streaming: None,
    })
}

// Sidekiq `MyWorker.perform_async(args)` / `.perform_later(args)` ActiveJob.
pub(crate) fn detect_ruby_bgjob_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !root
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
    {
        return None;
    }
    if !matches!(
        leaf,
        "perform_async" | "perform_later" | "perform_in" | "perform_at"
            | "enqueue" | "set" | "deliver_later"
    ) {
        return None;
    }
    // ActionMailer is handled by detect_ruby_actionmailer_emission.
    if root.ends_with("Mailer") {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::BgJob,
        name: format!("rb.{}", root),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

// Rails ActionMailer: `UserMailer.welcome(user).deliver_now`.
pub(crate) fn detect_ruby_actionmailer_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !root.ends_with("Mailer") || root == "Mailer" {
        return None;
    }
    if !matches!(
        leaf,
        "deliver_now" | "deliver_later" | "deliver" | "deliver_now!" | "deliver_later!"
    ) {
        return None;
    }
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::Mailer,
        name: format!("rb.{}", root),
        role: ChannelRole::Producer,
        method: None,
        streaming: None,
    })
}

// ActiveRecord `<Model>.where(...)`, `.find(...)`, `.create(...)`, etc.
// Chain root must be PascalCase (Ruby model convention); leaf op classifies.
pub(crate) fn detect_ruby_activerecord_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !is_pascal_case_first_rb(root) {
        return None;
    }
    let op = parse_activerecord_op(leaf)?;
    Some(FlowEmission::DbQuery {
        entity_name: format!("rb.{}", root),
        operation: op,
    })
}

fn parse_activerecord_op(name: &str) -> Option<crate::indexer::resolve::flow_emit::DbQueryOp> {
    use crate::indexer::resolve::flow_emit::DbQueryOp;
    Some(match name {
        "where" | "find" | "find_by" | "find_by!" | "find_each" | "first" | "last"
        | "all" | "count" | "exists?" | "pluck" | "select" | "includes" | "joins"
        | "left_joins" | "preload" | "eager_load" | "order" | "group" | "having"
        | "limit" | "offset" | "distinct" | "none" | "unscoped" | "merge" | "take"
        | "take!" | "any?" | "many?" | "average" | "minimum" | "maximum" | "sum"
        | "ids" | "size" | "length" | "to_a" => DbQueryOp::Select,
        "create" | "create!" | "insert" | "insert_all" | "insert_all!" => DbQueryOp::Insert,
        "update" | "update!" | "update_all" | "update_attributes" | "save" | "save!"
        | "upsert" | "upsert_all" | "touch" | "increment!" | "decrement!" => DbQueryOp::Update,
        "destroy" | "destroy!" | "destroy_all" | "delete" | "delete_all" => DbQueryOp::Delete,
        "find_or_create_by" | "find_or_create_by!" | "find_or_initialize_by" => DbQueryOp::Upsert,
        _ => return None,
    })
}

fn is_pascal_case_first_rb(name: &str) -> bool {
    name.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let require_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);

        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Gemfile) {
                let gem_root = require_path.split('/').next().unwrap_or(require_path);
                if manifest.dependencies.contains(gem_root)
                    || manifest.dependencies.contains(require_path)
                {
                    return Some(require_path.to_string());
                }
            }
        }

        if predicates::is_external_ruby_require(require_path, project_ctx) {
            return Some(require_path.to_string());
        }
        return None;
    }

    // Check file's require list for matching external gems.
    for import in &file_ctx.imports {
        let Some(module_path) = &import.module_path else {
            continue;
        };

        if module_path.starts_with('.') {
            continue;
        }

        if let Some(ctx) = project_ctx {
            if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&ManifestKind::Gemfile) {
                let gem_root = module_path.split('/').next().unwrap_or(module_path);
                if manifest.dependencies.contains(gem_root)
                    || manifest.dependencies.contains(module_path.as_str())
                {
                    return Some(module_path.clone());
                }
            }
        }

        if predicates::is_external_ruby_require(module_path, project_ctx) {
            return Some(module_path.clone());
        }
    }

    None
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;
    if r.kind == EdgeKind::Inherits {
        if let Some(emission) = detect_ruby_actioncable_emission(r.target_name.as_str()) {
            return vec![emission];
        }
        return Vec::new();
    }
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let Some(chain) = r.chain.as_ref() else {
        return Vec::new();
    };
    if let Some(emission) = detect_ruby_activerecord_emission(chain) {
        return vec![emission];
    }
    if let Some(emission) = detect_ruby_actionmailer_emission(chain) {
        return vec![emission];
    }
    if let Some(emission) = detect_ruby_bgjob_emission(chain) {
        return vec![emission];
    }
    if let Some(emission) = detect_ruby_http_emission(chain, &r.call_args) {
        return vec![emission];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    // Ruby extractor emits:
    //   require 'foo'          → target_name = "foo",  module = None (bare gem)
    //   require_relative './x' → target_name = "x",    module = "./x" (relative)
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

    // Add Gemfile manifest deps as wildcard imports — Ruby's require is
    // global-state-based, so any gem declared in the project is potentially
    // in scope. Covers transitive requires through helper files.
    if let Some(ctx) = project_ctx {
        let pkg_id = file.package_id;
        if let Some(manifest) = ctx.manifests_for(pkg_id).get(&ManifestKind::Gemfile) {
            for dep in &manifest.dependencies {
                let gem_root = dep.split('/').next().unwrap_or(dep.as_str());
                if !imports.iter().any(|i| {
                    i.module_path.as_deref().map(|m| m.split('/').next().unwrap_or(m)) == Some(gem_root)
                }) {
                    imports.push(ImportEntry {
                        imported_name: gem_root.to_string(),
                        module_path: Some(gem_root.to_string()),
                        alias: None,
                        is_wildcard: true,
                    });
                }
            }
        }
    }

    // Outermost module name extracted from the first Namespace symbol.
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

pub struct RubyHooks;

impl LanguageEngineHooks for RubyHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let _ = lookup;
        infer_external_inner(file_ctx, ref_ctx, project_ctx)
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

pub static RUBY_HOOKS: RubyHooks = RubyHooks;
