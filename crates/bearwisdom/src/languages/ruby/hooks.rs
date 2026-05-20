// =============================================================================
// languages/ruby/hooks.rs — RubyHooks impl plus the concrete RubyResolver
// (require/require_relative landing on indexed files via in_module_from,
// chain via RubyChecker, synthetic-global preference for ruby_stdlib /
// rubygems, scope-chain walk, same-file, same-module, fully-qualified-name
// with `::` ↔ `.` normalization, external gem lookup gated by imported gem
// list, `.rb`/`.rbs` bare-name fallback for open-class / mixin / monkey-
// patched methods) plus 5 flow detectors (Net::HTTP/Faraday/HTTParty/
// RestClient client calls, ActionCable consumer inheritance, Sidekiq /
// ActiveJob bgjob, ActionMailer deliver_*, ActiveRecord ORM ops) plus
// external classifier (Gemfile manifest deps + bare gem requires) plus
// build_file_context (require + require_relative + Gemfile transitive
// requires as wildcard imports).
// =============================================================================

use super::{predicates, type_checker::RubyChecker};
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct RubyResolver;

impl RubyResolver {
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

        // Imports: try to land the require on a real indexed file.
        if edge_kind == EdgeKind::Imports {
            if let Some(module) = &ref_ctx.extracted_ref.module {
                let syms = lookup.in_module_from(&file_ctx.file_path, module);
                // Prefer same-named module/class symbol when present
                // (`require "sidekiq/api"` → `Sidekiq::Api`); otherwise any
                // symbol in the file anchors the cross-file edge.
                let pick = syms
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(target))
                    .or_else(|| syms.first());
                if let Some(sym) = pick {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_require_file",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
            return None;
        }

        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            if let Some(res) = RubyChecker.resolve_chain(
                chain_val, edge_kind, None, ref_ctx, lookup,
            ) {
                return Some(res);
            }
        }

        // Synthetic-global lookup. ruby_stdlib / rubygems emit real symbols
        // for Kernel methods (puts, raise, lambda), Array/Hash/String
        // instance methods, and gemfile deps. ext:-only filter so scope /
        // same-file paths still win for project symbols.
        if ref_ctx.extracted_ref.chain.is_none() && !target.contains("::") {
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
                    strategy: "ruby_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Scope chain walk. Ruby uses `::` for namespacing in qualified
        // names; the index stores with `.` separator.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ruby_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_same_module",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        if target.contains("::") || target.contains('.') {
            let normalized = target.replace("::", ".");
            if let Some(sym) = lookup.by_qualified_name(&normalized) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
            if let Some(sym) = lookup.by_qualified_name(target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "ruby_qualified_name",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // External gem symbol lookup. When the project has external gem
        // sources indexed (origin='external'), match bare names against
        // external symbols gated by the file's imported gem set so a
        // method named in one gem doesn't bind to a similarly-named
        // method in an unrelated gem.
        {
            let candidates = lookup.by_name(target);
            let imported_gems: Vec<&str> = file_ctx
                .imports
                .iter()
                .filter_map(|imp| {
                    let m = imp.module_path.as_deref()?;
                    if m.starts_with('.') {
                        return None;
                    }
                    Some(m.split('/').next().unwrap_or(m))
                })
                .collect();

            for sym in candidates {
                if !sym.file_path.starts_with("ext:ruby:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let gem_name = sym.file_path
                    .strip_prefix("ext:ruby:")
                    .and_then(|rest| rest.split('/').next())
                    .unwrap_or("");
                if imported_gems.iter().any(|&g| g == gem_name || gem_name.starts_with(&format!("{g}-"))) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.8,
                        strategy: "ruby_external_gem",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        // Bare-name fallback. Ruby's open classes, monkey-patching, and
        // include/extend mixins put many methods in scope by bare name.
        // The engine's module/import path can't follow Ruby's runtime
        // composition.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains("::")
            && !target.contains('.')
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_ruby = path.ends_with(".rb") || path.ends_with(".rbs");
                if !is_ruby {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "ruby_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        None
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

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        RubyResolver.resolve(file_ctx, ref_ctx, lookup)
    }
}

pub static RUBY_HOOKS: RubyHooks = RubyHooks;
