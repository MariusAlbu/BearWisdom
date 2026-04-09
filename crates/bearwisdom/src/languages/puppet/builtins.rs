// =============================================================================
// puppet/builtins.rs — Puppet built-in resource types and functions
// =============================================================================

use crate::types::EdgeKind;

/// Edge-kind / symbol-kind compatibility for Puppet.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "class"),
        EdgeKind::TypeRef | EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Puppet core resource types (always provided by Puppet itself).
pub(super) fn is_puppet_resource_type(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "file"
            | "package"
            | "service"
            | "exec"
            | "user"
            | "group"
            | "host"
            | "cron"
            | "mount"
            | "notify"
            | "resources"
            | "augeas"
            | "computer"
            | "filebucket"
            | "interface"
            | "k5login"
            | "macauthorization"
            | "mailalias"
            | "maillist"
            | "mcx"
            | "nagios_command"
            | "nagios_contact"
            | "nagios_contactgroup"
            | "nagios_host"
            | "nagios_hostdependency"
            | "nagios_hostescalation"
            | "nagios_hostextinfo"
            | "nagios_hostgroup"
            | "nagios_service"
            | "nagios_servicedependency"
            | "nagios_serviceescalation"
            | "nagios_serviceextinfo"
            | "nagios_servicegroup"
            | "nagios_timeperiod"
            | "router"
            | "schedule"
            | "scheduled_task"
            | "selboolean"
            | "selmodule"
            | "ssh_authorized_key"
            | "sshkey"
            | "stage"
            | "tidy"
            | "vlan"
            | "whit"
            | "yumrepo"
            | "zfs"
            | "zone"
            | "zpool"
    )
}

/// Puppet built-in functions (always available without any module import).
pub(super) fn is_puppet_builtin_fn(name: &str) -> bool {
    matches!(
        name,
        // Class / resource management
        "include"
            | "require"
            | "contain"
            | "notify"
            | "realize"
            | "create_resources"
            | "ensure_resource"
            | "ensure_packages"
            | "defined"
            | "tagged"
            // Logging
            | "fail"
            | "warning"
            | "notice"
            | "info"
            | "debug"
            | "err"
            | "alert"
            | "emerg"
            | "crit"
            // Hiera / lookup
            | "lookup"
            | "hiera"
            | "hiera_array"
            | "hiera_hash"
            | "hiera_include"
            // Templates
            | "template"
            | "epp"
            | "inline_template"
            | "inline_epp"
            // File / generation
            | "file"
            | "generate"
            | "fqdn_rand"
            // Iteration
            | "each"
            | "map"
            | "filter"
            | "reduce"
            | "slice"
            | "with"
            | "reverse_each"
            | "any"
            | "all"
            // Type system
            | "assert_type"
            | "type"
            // Data manipulation
            | "dig"
            | "flatten"
            | "unique"
            | "sort"
            | "empty"
            | "size"
            | "length"
            | "keys"
            | "values"
            | "has_key"
            | "merge"
            | "delete"
            | "pick"
            | "pick_default"
            | "join"
            | "split"
            | "strip"
            | "chomp"
            | "downcase"
            | "upcase"
            | "capitalize"
            | "match"
            | "regsubst"
            | "sprintf"
            | "versioncmp"
            // Type predicates (stdlib)
            | "is_string"
            | "is_integer"
            | "is_float"
            | "is_numeric"
            | "is_bool"
            | "is_array"
            | "is_hash"
            // Validation (stdlib)
            | "validate_string"
            | "validate_array"
            | "validate_hash"
            | "validate_bool"
            | "validate_integer"
            | "validate_re"
    )
}

/// Combined check: true if the name is any kind of Puppet built-in.
pub(super) fn is_puppet_builtin(name: &str) -> bool {
    is_puppet_resource_type(name) || is_puppet_builtin_fn(name)
}
