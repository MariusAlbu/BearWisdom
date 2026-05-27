// Puppet language hooks. Absorbed from the deleted `puppet/resolve.rs`.

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct PuppetHooks;

fn is_puppet_global_var(name: &str) -> bool {
    let bare = name.strip_prefix('$').unwrap_or(name);
    let head = bare
        .split(|c: char| c == '.' || c == '[')
        .next()
        .unwrap_or(bare);
    matches!(
        head,
        "facts" | "trusted" | "server_facts" | "environment" | "servername" | "serverip"
            | "serverversion" | "clientcert" | "clientversion" | "clientnoop" | "module_name"
            | "caller_module_name" | "title" | "name" | "os" | "kernel" | "kernelrelease"
            | "kernelversion" | "operatingsystem" | "operatingsystemrelease" | "osfamily"
            | "lsbdistid" | "lsbdistdescription" | "lsbdistrelease" | "lsbdistcodename"
            | "architecture" | "hardwaremodel" | "processor0" | "processorcount"
            | "memorysize" | "memorytotal" | "fqdn" | "hostname" | "domain"
            | "ipaddress" | "ipaddress6" | "macaddress" | "interfaces" | "networking"
            | "path" | "pathseparator" | "puppetversion" | "rubyversion" | "rubysitedir"
            | "id" | "uptime" | "uptime_days" | "timezone"
    )
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    _project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;
    if is_puppet_global_var(target) {
        return Some("puppet-stdlib".to_string());
    }
    if let Some(prefix) = target.split("::").next() {
        let bare_prefix = prefix.strip_prefix('$').unwrap_or(prefix);
        if !bare_prefix.is_empty()
            && bare_prefix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            && target.contains("::")
        {
            let is_declared_forge = file_ctx
                .imports
                .iter()
                .any(|i| i.module_path.as_deref() == Some(bare_prefix));
            if is_declared_forge {
                return Some(format!("puppet_forge::{bare_prefix}"));
            }
            return Some(format!("puppet_module::{bare_prefix}"));
        }
    }
    if !target.is_empty()
        && !target.starts_with('$')
        && ref_ctx.extracted_ref.kind != EdgeKind::Imports
        && target
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
    {
        return Some("puppet_module::external".to_string());
    }
    None
}

impl LanguageEngineHooks for PuppetHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        if let Some(ctx) = project_ctx {
            if let Some(puppet) = ctx.manifest(ManifestKind::Puppet) {
                for module in &puppet.dependencies {
                    imports.push(ImportEntry {
                        imported_name: module.clone(),
                        module_path: Some(module.clone()),
                        alias: None,
                        is_wildcard: true,
                    });
                }
            }
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "puppet".to_string(),
            imports,
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if target.contains("::") {
            if let Some(prefix) = target.split("::").next() {
                let bare = prefix.strip_prefix('$').unwrap_or(prefix);
                if file_ctx
                    .imports
                    .iter()
                    .any(|i| i.module_path.as_deref() == Some(bare))
                {
                    return None;
                }
            }
        }
        if target.contains("::") {
            let last_segment = target.split("::").last().unwrap_or(target.as_str());
            for sym in lookup.in_file(&file_ctx.file_path) {
                if (sym.name == *target
                    || sym.name == last_segment
                    || sym.qualified_name == *target)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "puppet_qualified_same_file",
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
                        strategy: "puppet_qualified_global",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static PUPPET_HOOKS: PuppetHooks = PuppetHooks;
