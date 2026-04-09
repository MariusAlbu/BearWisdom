/// Puppet built-in resource types and built-in functions — always external
/// (never defined inside a project).
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Core resource types
    // -------------------------------------------------------------------------
    "file", "package", "service", "exec", "user", "group",
    "cron", "mount", "host", "notify", "tidy", "augeas",
    "yumrepo", "selboolean", "selmodule", "mailalias",
    "ssh_authorized_key", "sshkey",
    "nagios_host", "nagios_service",
    // -------------------------------------------------------------------------
    // Built-in functions — class / resource management
    // -------------------------------------------------------------------------
    "include", "require", "contain", "realize",
    "create_resources", "ensure_resource", "ensure_packages",
    "defined", "tagged",
    // -------------------------------------------------------------------------
    // Logging
    // -------------------------------------------------------------------------
    "fail", "warning", "notice", "info", "debug",
    "err", "alert", "emerg", "crit",
    // -------------------------------------------------------------------------
    // Hiera / lookup
    // -------------------------------------------------------------------------
    "lookup", "hiera", "hiera_array", "hiera_hash", "hiera_include",
    // -------------------------------------------------------------------------
    // Templates
    // -------------------------------------------------------------------------
    "template", "epp", "inline_template", "inline_epp",
    // -------------------------------------------------------------------------
    // File / generation
    // -------------------------------------------------------------------------
    "generate", "fqdn_rand",
    // -------------------------------------------------------------------------
    // Iteration
    // -------------------------------------------------------------------------
    "each", "map", "filter", "reduce", "slice", "with",
    "assert_type", "type", "dig", "flatten", "unique",
    "sort", "reverse_each", "any", "all",
    "empty", "size", "length",
    "keys", "values", "has_key", "merge", "delete",
    "pick", "pick_default",
    "join", "split", "strip", "chomp",
    "downcase", "upcase", "capitalize",
    "match", "regsubst", "sprintf", "versioncmp",
    // -------------------------------------------------------------------------
    // Type predicates / validation (stdlib)
    // -------------------------------------------------------------------------
    "is_string", "is_integer", "is_float", "is_numeric",
    "is_bool", "is_array", "is_hash",
    "validate_string", "validate_array", "validate_hash",
    "validate_bool", "validate_integer", "validate_re",
];
