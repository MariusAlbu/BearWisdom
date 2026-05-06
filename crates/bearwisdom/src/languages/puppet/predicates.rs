// =============================================================================
// puppet/predicates.rs — edge-kind compatibility + forge-module classification
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

/// Well-known Puppet Forge module prefixes. When a qualified class name such as
/// `stdlib::validate_integer` or `apache::vhost` begins with one of these
/// prefixes, the reference points to a forge dependency, not a project symbol.
pub(super) fn is_forge_module(prefix: &str) -> bool {
    matches!(
        prefix,
        // puppetlabs modules (stdlib, apache, mysql, etc.)
        "stdlib"
            | "apache"
            | "mysql"
            | "postgresql"
            | "concat"
            | "apt"
            | "firewall"
            | "vcsrepo"
            | "java"
            | "tomcat"
            | "nginx"
            | "haproxy"
            | "ntp"
            | "sshd"
            | "sudo"
            | "motd"
            | "limits"
            | "sysctl"
            | "timezone"
            | "accounts"
            | "archive"
            | "augeas"
            | "cron"
            | "docker"
            | "git"
            | "inifile"
            | "java_ks"
            | "lvm"
            | "mongodb"
            | "rabbitmq"
            | "redis"
            | "rsync"
            | "swap_file"
            | "xinetd"
    )
}
