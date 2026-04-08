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
// =============================================================================

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

        // Built-in Puppet resource types never live in the project index.
        if is_puppet_builtin(target) {
            return None;
        }

        engine::resolve_common("puppet", file_ctx, ref_ctx, lookup, puppet_kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // Combine both builtin predicates into one for the common helper.
        engine::infer_external_common(file_ctx, ref_ctx, |name| {
            is_puppet_builtin(name) || is_puppet_builtin_function(name)
        })
        .map(|_| "puppet".to_string())
    }
}

/// Puppet built-in resource types (core resource types provided by Puppet itself).
fn is_puppet_builtin(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        // Core resource types
        "file" | "package" | "service" | "exec" | "user" | "group"
            | "host" | "cron" | "mount" | "notify" | "resources"
            | "augeas" | "computer" | "filebucket" | "interface"
            | "k5login" | "macauthorization" | "mailalias" | "maillist"
            | "mcx" | "nagios_command" | "nagios_contact" | "nagios_contactgroup"
            | "nagios_host" | "nagios_hostdependency" | "nagios_hostescalation"
            | "nagios_hostextinfo" | "nagios_hostgroup" | "nagios_service"
            | "nagios_servicedependency" | "nagios_serviceescalation"
            | "nagios_serviceextinfo" | "nagios_servicegroup" | "nagios_timeperiod"
            | "router" | "schedule" | "scheduled_task" | "selboolean"
            | "selmodule" | "ssh_authorized_key" | "sshkey" | "stage"
            | "tidy" | "vlan" | "whit" | "yumrepo" | "zfs" | "zone" | "zpool"
    )
}

/// Edge-kind / symbol-kind compatibility for Puppet.
fn puppet_kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "class"),
        EdgeKind::TypeRef | EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Puppet built-in functions.
fn is_puppet_builtin_function(name: &str) -> bool {
    matches!(
        name,
        "alert" | "assert_type" | "contain" | "create_resources" | "debug"
            | "defined" | "digest" | "each" | "emerg" | "err" | "fail"
            | "file" | "filter" | "fqdn_rand" | "generate" | "hiera"
            | "hiera_array" | "hiera_hash" | "hiera_include" | "include"
            | "info" | "inline_epp" | "inline_template" | "lookup"
            | "map" | "md5" | "notice" | "realize" | "reduce"
            | "regsubst" | "require" | "sha1" | "slice" | "sprintf"
            | "split" | "strftime" | "tag" | "tagged" | "template"
            | "versioncmp" | "warning" | "with"
    )
}
