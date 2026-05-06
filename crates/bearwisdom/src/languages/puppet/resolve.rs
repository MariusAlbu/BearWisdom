// =============================================================================
// languages/puppet/resolve.rs — Puppet resolution rules
//
// Puppet references:
//
//   include apache::config            → Calls, target_name = "apache::config"
//   require nginx                     → Calls, target_name = "nginx"
//   class { 'myapp::web': }           → TypeRef, target_name = "myapp::web"
//   file { '/etc/app.conf': }         → TypeRef (built-in resource type)
//   Class['apache']                   → TypeRef, target_name = "apache"
//
// Resolution strategy:
//   1. Same-file: classes and defined types in the same manifest.
//   2. Import-based: Puppet autoloads classes from the module path using the
//      `::` namespace separator. `apache::config` maps to
//      `apache/manifests/config.pp`.
//   3. Global name lookup (cross-file).
//   4. Built-in Puppet resource types are external.
//   5. Forge module references: first `::` segment is a known forge module →
//      classified as external without index lookup.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct PuppetResolver;

impl LanguageResolver for PuppetResolver {
    fn language_ids(&self) -> &[&str] {
        &["puppet"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // Puppet uses autoloading based on the `::` namespace — no explicit
        // import statements to collect.
        FileContext {
            file_path: file.path.clone(),
            language: "puppet".to_string(),
            imports: Vec::new(),
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

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Bare-name lookup with synthetic-symbol preference. Puppet stdlib
        // and core resource types are synthesised under `ext:puppet-stdlib:`
        // and `ext:puppet-forge:` paths; binding to those gives a real
        // resolved edge. Run BEFORE the forge-module shortcut so a bare
        // name like `concat` (which is both a forge module *and* a stdlib
        // function) binds to the function symbol instead of being misrouted
        // to external classification with no symbol attached.
        //
        // Also case-insensitive: Puppet defined-type references capitalize
        // (`Myfile[$x]`) while the `define myfile($p)` declaration is
        // lowercase. Without folding, every TypeRef to a same-project
        // defined type stays unresolved.
        if !target.contains("::") {
            let target_lower = target.to_ascii_lowercase();
            let mut synthetic_match = None;
            let mut internal_match = None;
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                if sym.file_path.starts_with("ext:") {
                    synthetic_match = Some(sym);
                    break;
                } else if internal_match.is_none() {
                    internal_match = Some(sym);
                }
            }
            if synthetic_match.is_none() && internal_match.is_none() && **target != target_lower {
                for sym in lookup.by_name(&target_lower) {
                    if !predicates::kind_compatible(edge_kind, &sym.kind) {
                        continue;
                    }
                    if sym.file_path.starts_with("ext:") {
                        synthetic_match = Some(sym);
                        break;
                    } else if internal_match.is_none() {
                        internal_match = Some(sym);
                    }
                }
            }
            if let Some(sym) = synthetic_match.or(internal_match) {
                let strategy = if sym.file_path.starts_with("ext:") {
                    "puppet_synthetic_global"
                } else {
                    "puppet_internal_global"
                };
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: if strategy == "puppet_synthetic_global" { 0.95 } else { 0.9 },
                    strategy,
                    resolved_yield_type: None,
                });
            }
        }

        // Forge module references (qualified `<module>::<class>`) are
        // external — skip further index lookup. Plain bare names already
        // had their chance via the synthetic lookup above.
        if target.contains("::") {
            if let Some(prefix) = target.split("::").next() {
                let bare = prefix.strip_prefix('$').unwrap_or(prefix);
                if predicates::is_forge_module(bare) {
                    return None;
                }
            }
        }

        // For qualified names with `::`, the extractor stores the full name as
        // `target_name` (e.g. "profile::base"). `resolve_common` step 5 handles
        // `target.contains("::")` via `by_qualified_name`. We additionally try
        // matching just the last segment in the same file (for locally-defined
        // classes whose qualified_name was recorded without the module prefix).
        if target.contains("::") {
            let last_segment = target.split("::").last().unwrap_or(target.as_str());
            for sym in lookup.in_file(&file_ctx.file_path) {
                if (sym.name == *target || sym.name == last_segment || sym.qualified_name == *target)
                    && predicates::kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "puppet_qualified_same_file",
                        resolved_yield_type: None,
                    });
                }
            }

            // Cross-file: try exact qualified name, then just last segment
            // (handles classes declared without module prefix in their own file).
            if let Some(sym) = lookup.by_qualified_name(target) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "puppet_qualified_global",
                        resolved_yield_type: None,
                    });
                }
            }
            for sym in lookup.by_name(last_segment) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: "puppet_unqualified_fallback",
                        resolved_yield_type: None,
                    });
                }
            }
        }

        engine::resolve_common("puppet", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Puppet built-in global variables: `$facts`, `$trusted`, `$server_facts`,
        // etc. are always available without declaration.
        if is_puppet_global_var(target) {
            return Some("puppet-stdlib".to_string());
        }

        // Any `<prefix>::<rest>` reference that reached infer_external_namespace
        // (i.e. resolve already failed to find a project symbol for it) belongs
        // to a module Puppet would have auto-loaded from the module path. Classify
        // it as external under the prefix's namespace — the prefix is the module
        // name in every real Puppet codebase. Forge prefixes get their own bucket;
        // unknown prefixes share `puppet_forge::<prefix>` so cross-project
        // dashboards can still group them.
        if let Some(prefix) = target.split("::").next() {
            // Strip a leading `$` for variable refs like `$mysql::server::opt` —
            // the prefix is `mysql`, the rest is a class-scoped variable.
            let bare_prefix = prefix.strip_prefix('$').unwrap_or(prefix);
            if !bare_prefix.is_empty()
                && bare_prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && target.contains("::")
            {
                if predicates::is_forge_module(bare_prefix) {
                    return Some(format!("puppet_forge::{bare_prefix}"));
                }
                return Some(format!("puppet_module::{bare_prefix}"));
            }
        }

        // Bare-name fall-through: Puppet auto-loads classes, defined types,
        // and functions from the module path. A reference that reached this
        // point exhausted resolve()'s same-file, synthetic, qualified, and
        // global lookups — Puppet's loader would have searched the module
        // path next, so classify as external rather than leave unresolved.
        // Skip variables (start with `$`) since those genuinely indicate a
        // missing local binding the maintainer should see.
        if !target.is_empty()
            && !target.starts_with('$')
            && ref_ctx.extracted_ref.kind != EdgeKind::Imports
            && target.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false)
        {
            return Some("puppet_module::external".to_string());
        }
        None
    }
}

/// Puppet's built-in top-level variables, always in scope without declaration.
/// See: https://puppet.com/docs/puppet/latest/lang_facts_and_builtin_vars.html
fn is_puppet_global_var(name: &str) -> bool {
    let bare = name.strip_prefix('$').unwrap_or(name);
    // Allow member access: `$facts['foo']` arrives as `$facts`, but `$facts.foo`
    // is extracted as `$facts` too; just match the head.
    let head = bare.split(|c: char| c == '.' || c == '[').next().unwrap_or(bare);
    matches!(
        head,
        "facts"
            | "trusted"
            | "server_facts"
            | "environment"
            | "servername"
            | "serverip"
            | "serverversion"
            | "clientcert"
            | "clientversion"
            | "clientnoop"
            | "module_name"
            | "caller_module_name"
            | "title"
            | "name"
            // Top-level fact aliases that Puppet 4+ exposes alongside `$facts['<name>']`.
            // Without them, every `$os.family` in modern manifests stays unresolved.
            | "os"
            | "kernel"
            | "kernelrelease"
            | "kernelversion"
            | "operatingsystem"
            | "operatingsystemrelease"
            | "osfamily"
            | "lsbdistid"
            | "lsbdistdescription"
            | "lsbdistrelease"
            | "lsbdistcodename"
            | "architecture"
            | "hardwaremodel"
            | "processor0"
            | "processorcount"
            | "memorysize"
            | "memorytotal"
            | "fqdn"
            | "hostname"
            | "domain"
            | "ipaddress"
            | "ipaddress6"
            | "macaddress"
            | "interfaces"
            | "networking"
            | "path"
            | "pathseparator"
            | "puppetversion"
            | "rubyversion"
            | "rubysitedir"
            | "id"
            | "uptime"
            | "uptime_days"
            | "timezone"
    )
}

