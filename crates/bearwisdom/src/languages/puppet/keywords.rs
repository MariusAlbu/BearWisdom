// =============================================================================
// puppet/keywords.rs — Puppet primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Puppet.
pub(crate) const KEYWORDS: &[&str] = &[
    // resource ensure values
    "ensure", "present", "absent", "installed", "running",
    "stopped", "enabled", "disabled", "latest", "purged",
    // metaparameters
    "file", "directory", "link", "notify", "require",
    "before", "subscribe",
    // declarations
    "include", "contain", "class", "define", "node",
    "site", "application", "inherits",
    // literals
    "undef", "true", "false", "default",
    // flow
    "case", "if", "elsif", "else", "unless",
    "each", "map", "filter", "reduce", "slice", "with",
    // hiera / data
    "lookup", "hiera", "hiera_hash", "hiera_array", "hiera_include",
    "create_resources", "defined",
    // logging
    "fail", "warning", "notice", "info", "debug", "alert", "crit", "emerg", "err",
    // string functions
    "regsubst", "split", "join", "size", "length", "empty",
    "flatten", "unique", "sort", "reverse", "member", "any", "all",
    "dig", "values", "keys", "has_key", "merge", "delete",
    "pick", "pick_default", "assert_type", "type", "versioncmp",
    // templates
    "template", "epp", "inline_template", "inline_epp",
    // misc functions
    "fqdn_rand", "generate",
    "md5", "sha1", "sha256", "base64",
    // stdlib validate (legacy)
    "validate_string", "validate_integer", "validate_bool",
    "validate_array", "validate_hash", "validate_re",
    "is_string", "is_integer", "is_float", "is_numeric",
    "is_bool", "is_array", "is_hash",
    "is_ip_address", "is_domain_name", "is_function_available",
    "str2bool", "bool2str", "num2bool", "bool2num",
    // numeric
    "abs", "max", "min", "ceil", "floor", "round", "range",
    // string extras
    "chomp", "chop", "capitalize", "downcase", "upcase",
    "strip", "lstrip", "rstrip", "shellquote", "uriescape", "pw_hash",
    "parsejson", "parseyaml", "to_json", "to_yaml", "to_json_pretty",
    "sprintf",
    // Puppet type system
    "Integer", "Float", "Boolean", "String", "Array", "Hash",
    "Regexp", "Undef", "Default", "Optional", "Variant",
    "Enum", "Pattern", "Struct", "Tuple", "Callable",
    "Type", "Any", "Data", "Scalar", "Numeric", "Collection",
    "Catalogentry", "Resource", "Class", "TypeFactory", "Error",
    // common access
    "Puppet", "Facter",
    // built-in resource types
    "File", "Package", "Service", "User", "Group", "Exec",
    "Cron", "Mount", "Tidy", "Host", "Mailalias",
    "Ssh_authorized_key", "Sshkey", "Schedule", "Stage",
    "Filebucket", "Resources",
    // rspec-puppet matchers
    "expect", "it", "describe", "context", "subject",
    "let", "before", "after",
    "eq", "match", "include", "contain", "be", "have", "receive", "allow",
    "raise_error", "a", "an", "be_a", "be_an", "satisfy",
    "eql", "equal", "be_truthy", "be_falsey", "be_nil", "be_empty",
    "be_between", "respond_to", "start_with", "end_with",
    "change", "output", "cover",
];
