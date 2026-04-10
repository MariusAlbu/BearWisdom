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
    // -------------------------------------------------------------------------
    // Puppet type system — built-in data types
    // -------------------------------------------------------------------------
    "Variant", "Optional", "Enum", "Pattern", "Regexp",
    "Tuple", "Struct", "Hash", "Array",
    "Integer", "Float", "String", "Boolean",
    "Undef", "Any", "Callable", "Type",
    "CatalogEntry", "Resource", "Class",
    // -------------------------------------------------------------------------
    // Stdlib custom types
    // -------------------------------------------------------------------------
    "Stdlib::Absolutepath", "Stdlib::Filesource", "Stdlib::Port",
    "Stdlib::IP::Address",
];

/// RSpec and rspec-puppet globals present in Puppet module spec files.
/// These appear as bare identifiers in `spec/` directories and are never
/// defined by the module itself.
const RSPEC_GLOBALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // RSpec core — example group / example definition
    // -------------------------------------------------------------------------
    "describe", "context", "it", "specify",
    "before", "after", "around",
    "let", "let!", "subject",
    "shared_examples", "shared_context",
    "include_examples", "include_context", "it_behaves_like",
    // -------------------------------------------------------------------------
    // RSpec expectations — entry points
    // -------------------------------------------------------------------------
    "expect", "allow", "receive", "have_received",
    "is_expected", "should", "should_not",
    // -------------------------------------------------------------------------
    // RSpec matchers
    // -------------------------------------------------------------------------
    "eq", "eql", "equal",
    "be", "be_truthy", "be_falsey", "be_nil",
    "be_a", "be_an", "be_instance_of",
    "include", "match", "match_array", "contain_exactly",
    "start_with", "end_with",
    "raise_error", "change", "satisfy",
    // -------------------------------------------------------------------------
    // rspec-puppet matchers
    // -------------------------------------------------------------------------
    "compile",
    "contain_class", "contain_file", "contain_package",
    "contain_service", "contain_exec", "contain_notify",
    "with", "without",
    "that_comes_before", "that_requires", "that_notifies", "that_subscribes_to",
    "have_resource_count", "catalogue",
];

/// Returns framework-injected globals for Puppet.
///
/// RSpec / rspec-puppet identifiers are unconditional — any Puppet module that
/// ships a `spec/` directory uses them, and there is no Gemfile dependency to
/// inspect at resolution time.
pub(crate) fn framework_globals(
    _deps: &std::collections::HashSet<String>,
) -> Vec<&'static str> {
    RSPEC_GLOBALS.to_vec()
}
