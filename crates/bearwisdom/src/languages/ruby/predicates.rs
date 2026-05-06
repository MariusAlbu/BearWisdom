// =============================================================================
// ruby/predicates.rs — Ruby builtin and helper predicates
// =============================================================================

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        // Ruby modules (mixins) are stored as "namespace" in the index.
        EdgeKind::Implements => matches!(sym_kind, "namespace" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "namespace" | "interface" | "enum" | "type_alias"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Ruby stdlib module names — always external regardless of Gemfile.
const RUBY_STDLIB: &[&str] = &[
    "json",
    "net/http",
    "uri",
    "fileutils",
    "set",
    "csv",
    "yaml",
    "erb",
    "cgi",
    "digest",
    "base64",
    "open-uri",
    "socket",
    "logger",
    "optparse",
    "benchmark",
    "tempfile",
    "pathname",
    "date",
    "time",
    "pp",
    "forwardable",
    "singleton",
    "ostruct",
    "struct",
];

/// Check whether a require path refers to an external gem or stdlib.
pub(super) fn is_external_ruby_require(
    require_path: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    // Stdlib — always external.
    if RUBY_STDLIB.contains(&require_path) {
        return true;
    }
    // Check gem names from Gemfile manifest.
    if let Some(ctx) = project_ctx {
        let gem_root = require_path.split('/').next().unwrap_or(require_path);
        if ctx.has_dependency(ManifestKind::Gemfile, gem_root)
            || ctx.has_dependency(ManifestKind::Gemfile, require_path)
        {
            return true;
        }
    }
    false
}
