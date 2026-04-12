// =============================================================================
// ruby/externals.rs — Ruby runtime globals and framework-injected names
// =============================================================================

/// Ruby runtime globals that are always external.
///
/// These identifiers appear in Ruby code but are never defined in project
/// source — they are kernel-level, stdlib-level, or interpreter globals.
pub(crate) const EXTERNALS: &[&str] = &[
    // Special variables / pseudo-globals
    "__method__",
    "__dir__",
    "__callee__",
    // Kernel-injected globals
    "$stdout",
    "$stderr",
    "$stdin",
    "$0",
    "$PROGRAM_NAME",
    "$LOAD_PATH",
    "$LOADED_FEATURES",
    "$:",
    "$\"",
    // Ruby stdlib type constants — require'd from stdlib but used as bare names
    "SecureRandom",
    "JSON",
    "URI",
    "Pathname",
    "FileUtils",
    "Tempfile",
    "StringIO",
    "OpenStruct",
    "Set",
    "Mutex",
    "Process",
    "BigDecimal",
    "Logger",
    "Date",
    "DateTime",
    "Time",
    "Regexp",
    // Additional stdlib constants used without explicit require in Rails apps
    "Base64",
    "Digest",
    "CSV",
    "YAML",
    "ERB",
    "CGI",
    "Net",
    "Socket",
    "Encoding",
    "Math",
    "Comparable",
];

