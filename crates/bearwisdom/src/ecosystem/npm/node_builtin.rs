// =============================================================================
// ecosystem/npm/node_builtin — Node builtin specifiers and supplied types
//
// Node's `node:` module scheme is TypeScript/JavaScript-specific import
// evidence. Its declarations are supplied by the TypeScript toolchain under
// the canonical `@types/node` virtual root, never by a project file or an
// arbitrary npm package. Keeping this policy with the npm ecosystem gives
// generic resolver code one narrow, fail-closed adapter surface.
// =============================================================================

/// Canonical virtual-path prefix for the Node declaration source supplied by
/// the TypeScript toolchain.
pub(crate) const NODE_TYPES_VIRTUAL_ROOT: &str = "ext:ts:@types/node/";

/// The bare Node builtin name behind a well-formed `node:` specifier.
///
/// `node:path` becomes `path`; `node:fs/promises` becomes `fs/promises`.
/// Each segment is intentionally identifier-like. Traversals, empty segments,
/// and nested URI schemes retain their raw spelling and gain no supplied-type
/// authority.
pub(crate) fn node_builtin_module_alias(specifier: &str) -> Option<&str> {
    let alias = specifier.strip_prefix("node:")?;
    if alias.is_empty()
        || alias.split('/').any(|segment| {
            segment.is_empty()
                || matches!(segment, "." | "..")
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        })
    {
        return None;
    }
    Some(alias)
}

/// Whether `path` is a declaration under the canonical supplied `@types/node`
/// root. The virtual-path convention uses `/`, but normalize a legacy Windows
/// spelling before checking so authorization cannot depend on host separators.
pub(crate) fn is_node_builtin_declaration_path(path: &str) -> bool {
    path.replace('\\', "/").starts_with(NODE_TYPES_VIRTUAL_ROOT)
}

const TYPESCRIPT_DECLARATION_EXTENSIONS: &[&str] = &[".d.ts", ".d.mts", ".d.cts"];

/// Build the generic path evidence for a TypeScript/JavaScript module.
/// Ordinary modules retain heuristic matching while using declaration-file
/// suffixes as one extension. A valid `node:` builtin is fenced to its supplied
/// declaration root; a malformed one is rejected without fallback.
pub(crate) fn module_path_match(
    specifier: &str,
) -> crate::type_checker::profile::language_profile::ModulePathMatch {
    use crate::type_checker::profile::language_profile::{
        ModuleMatchAuthority, ModulePathMatch,
    };

    if let Some(alias) = node_builtin_module_alias(specifier) {
        return ModulePathMatch {
            module_path: alias.to_string(),
            required_file_prefix: Some(NODE_TYPES_VIRTUAL_ROOT),
            compound_extensions: TYPESCRIPT_DECLARATION_EXTENSIONS,
            authority: ModuleMatchAuthority::Authoritative,
        };
    }
    if specifier.starts_with("node:") {
        return ModulePathMatch {
            module_path: specifier.to_string(),
            required_file_prefix: None,
            compound_extensions: TYPESCRIPT_DECLARATION_EXTENSIONS,
            authority: ModuleMatchAuthority::Reject,
        };
    }
    ModulePathMatch {
        module_path: specifier.to_string(),
        required_file_prefix: None,
        compound_extensions: TYPESCRIPT_DECLARATION_EXTENSIONS,
        authority: ModuleMatchAuthority::Heuristic,
    }
}

#[cfg(test)]
#[path = "node_builtin_tests.rs"]
mod tests;
