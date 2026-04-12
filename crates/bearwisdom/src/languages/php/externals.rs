// =============================================================================
// php/externals.rs — PHP framework-injected global names
//
// PHP projects rely heavily on framework-provided helper functions that are
// globally available after a framework package is installed (Laravel's
// `route()`, `view()`, `config()`, `trans()`, Symfony's `dump()`, WordPress's
// `get_option()`, etc.). These helpers never appear as source-defined symbols
// in the project — they live inside the framework's own package, usually in
// a global `helpers.php` loaded at boot time.
//
// Two tiers:
//   EXTERNALS     — always-external identifiers (built-in PHP superglobals,
//                   common library globals that PHP projects universally use).
//   framework_globals — dep-gated. Only active when the corresponding
//                   composer package appears in composer.json `require`.
// =============================================================================

/// Always-external PHP names. Covers PHP superglobals and near-universal
/// vendor libraries whose identifiers should never resolve to project code.
pub(crate) const EXTERNALS: &[&str] = &[
    // PHP superglobals (arrays) — referenced as identifiers in many projects
    "_GET",
    "_POST",
    "_REQUEST",
    "_SERVER",
    "_SESSION",
    "_COOKIE",
    "_FILES",
    "_ENV",
    "GLOBALS",
    // Common Monolog / PSR logger facades that appear as method-call
    // receivers without corresponding use statements
    "LoggerInterface",
];

