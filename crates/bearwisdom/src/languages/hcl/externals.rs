// =============================================================================
// hcl/externals.rs — Terraform / HCL external symbol list
// =============================================================================

/// Terraform built-in functions and meta-argument names that are always
/// external — they will never resolve to a project-defined symbol.
pub(crate) const EXTERNALS: &[&str] = &[
    // -----------------------------------------------------------------------
    // Numeric
    // -----------------------------------------------------------------------
    "abs", "ceil", "floor", "log", "max", "min", "parseint", "pow", "signum",
    // -----------------------------------------------------------------------
    // String
    // -----------------------------------------------------------------------
    "chomp", "endswith", "format", "formatlist", "indent", "join",
    "lower", "ltrim", "regex", "regexall", "replace", "rtrim",
    "split", "startswith", "strcontains", "strrev", "substr",
    "templatestring", "title", "trim", "trimprefix", "trimsuffix",
    "trimspace", "upper",
    // -----------------------------------------------------------------------
    // Collection
    // -----------------------------------------------------------------------
    "alltrue", "anytrue", "chunklist", "coalesce", "coalescelist",
    "compact", "concat", "contains", "distinct", "element", "flatten",
    "index", "keys", "length", "list", "lookup", "map", "matchkeys",
    "merge", "one", "range", "reverse", "setintersection", "setproduct",
    "setsubtract", "setunion", "slice", "sort", "sum",
    "tolist", "tomap", "toset", "transpose", "values", "zipmap",
    // -----------------------------------------------------------------------
    // Encoding
    // -----------------------------------------------------------------------
    "base64decode", "base64encode", "base64gzip", "csvdecode",
    "jsondecode", "jsonencode", "textdecodebase64", "textencodebase64",
    "urlencode", "yamldecode", "yamlencode",
    // -----------------------------------------------------------------------
    // Filesystem
    // -----------------------------------------------------------------------
    "abspath", "dirname", "pathexpand", "basename", "file",
    "fileexists", "fileset", "filebase64",
    "filebase64sha256", "filebase64sha512", "filemd5",
    "filesha1", "filesha256", "filesha512", "templatefile",
    // -----------------------------------------------------------------------
    // Date / time
    // -----------------------------------------------------------------------
    "formatdate", "plantimestamp", "timeadd", "timecmp", "timestamp",
    // -----------------------------------------------------------------------
    // Hash / crypto
    // -----------------------------------------------------------------------
    "base64sha256", "base64sha512", "bcrypt", "md5", "rsadecrypt",
    "sha1", "sha256", "sha512", "uuid", "uuidv5",
    // -----------------------------------------------------------------------
    // IP / networking
    // -----------------------------------------------------------------------
    "cidrhost", "cidrnetmask", "cidrsubnet", "cidrsubnets",
    // -----------------------------------------------------------------------
    // Type conversion / introspection
    // -----------------------------------------------------------------------
    "can", "issensitive", "nonsensitive", "sensitive",
    "tobool", "tonumber", "tostring", "try", "type",
    // -----------------------------------------------------------------------
    // Type keywords
    // -----------------------------------------------------------------------
    "string", "number", "bool", "any", "object", "tuple",
    // -----------------------------------------------------------------------
    // Meta-arguments / special references
    // -----------------------------------------------------------------------
    "count", "for_each", "depends_on", "provider", "lifecycle",
    "provisioner", "connection", "each", "path", "self", "terraform",
];
