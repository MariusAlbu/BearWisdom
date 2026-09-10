// =============================================================================
// python/predicates.rs — edge-kind compatibility + relative-import shape check
// =============================================================================

use crate::type_checker::profile::language_profile::{
    ModuleSpecifierClass, SourceModulePathPolicy,
};
use crate::types::EdgeKind;

pub(crate) const SOURCE_MODULE_PATH_POLICY: SourceModulePathPolicy = SourceModulePathPolicy {
    classify_specifier: classify_module_specifier,
    relative_candidate_paths: |base| vec![base.to_string(), format!("{base}.py")],
    bare_module_matches_file: |_file, _module| false,
    external_import_match_terms: |_| Vec::new(),
};

/// Python keeps explicit relative imports and filesystem-rooted module paths
/// separate from dotted package imports.
fn classify_module_specifier(specifier: &str) -> ModuleSpecifierClass {
    if specifier.starts_with('.')
        || specifier.starts_with('/')
        || (specifier.len() >= 2 && specifier.as_bytes()[1] == b':')
    {
        ModuleSpecifierClass::Relative
    } else if specifier.is_empty() {
        ModuleSpecifierClass::Unsupported
    } else {
        ModuleSpecifierClass::Bare
    }
}

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "class"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// A relative import starts with a dot (`./`, `../`) or is an internal module
/// path (no domain-style host segment, not a stdlib name).
///
/// We approximate: if the module path starts with `.` or contains a `/` it's
/// relative/local. Otherwise it might be an installed package.
pub(super) fn is_relative_import(module: &str) -> bool {
    module.starts_with('.') || module.starts_with('/')
}

/// A name in Python's `builtins` module surface — a bare reference to it is
/// a language construct, never a missing project symbol, regardless of
/// whether it appears as a `Calls` target or a `TypeRef` target. Wired as
/// the profile's `builtin_skip` gate.
pub(super) fn is_python_builtin(name: &str) -> bool {
    super::keywords::BUILTIN_NAMES.contains(&name)
}
