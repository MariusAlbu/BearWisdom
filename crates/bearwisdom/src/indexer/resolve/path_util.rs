// =============================================================================
// indexer/resolve/path_util.rs — file-path predicates for the heuristic resolver
//
// Small, stateless helpers used by the tier-2 heuristic strategies to compare
// reference paths against indexed file paths: parent-directory extraction,
// module-entry-point detection (`mod.rs` / `index.ts` / `__init__.py`),
// path-vs-module matching, kind compatibility, and the `ext:` external-
// path predicate.
// =============================================================================

use crate::types::EdgeKind;

/// Check whether a file path "matches" a module reference.
///
/// Return the parent directory portion of a file path (everything up to the last `/`).
pub(super) fn parent_dir(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
}

/// Check if a file is a module entry point (public re-export surface).
pub(super) fn is_module_entry_point(path: &str) -> bool {
    let basename = path.rsplit('/').next().unwrap_or(path);
    matches!(
        basename,
        "mod.rs"
            | "lib.rs"
            | "__init__.py"
            | "index.ts"
            | "index.tsx"
            | "index.js"
            | "index.jsx"
            | "index.mts"
            | "index.mjs"
    )
}

/// Handles both:
///   - TS relative imports: `./catalog` matches `src/catalog.ts`
///   - C# namespace: `System.Linq` matches a file in namespace `System`
pub(super) fn file_path_matches_module(file_path: &str, module: &str) -> bool {
    if module.is_empty() {
        return false;
    }
    // Relative TS path: strip leading `./` and common extensions.
    let module_clean = module.trim_start_matches("./").trim_start_matches("../");

    // Try suffix match (e.g., "catalog" matches "src/catalog.ts").
    let file_stem = file_path
        .trim_end_matches(".ts")
        .trim_end_matches(".tsx")
        .trim_end_matches(".js")
        .trim_end_matches(".cs");

    if file_stem.ends_with(module_clean) || file_stem.ends_with(&module_clean.replace('.', "/")) {
        return true;
    }

    // C# namespace: "System.Linq" — check if file_path contains "Linq" as a path component.
    let last_segment = module.rsplit('.').next().unwrap_or(module);
    file_path.contains(last_segment)
}

/// Returns true when `sym_kind` is a plausible target for the given edge kind.
///
/// Used by P4 to prefer kind-compatible candidates over incidental name
/// collisions (e.g. a `Calls` ref should prefer a method over a class).
pub(super) fn kind_matches_symbol_kind(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        // `property` accepted: TypeScript `.d.ts` interface members like
        // `dispatchEvent(event: Event): boolean` parse as `property_signature`
        // and extract as kind="property" but are callable at runtime. DOM
        // methods (`querySelector`, `getAttribute`, `dispatchEvent`), Function
        // prototype methods (`bind`, `apply`, `call`), and similar interface-
        // declared methods all land here.
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class"
                | "struct"
                | "interface"
                | "enum"
                | "enum_member"
                | "type_alias"
                | "namespace"
                | "delegate"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        // Imports, HttpCall, DbEntity, LspResolved — accept any kind.
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Index builders
// ---------------------------------------------------------------------------

/// Build a map from simple name → list of (file_path, qualified_name, kind, symbol_id).
///
/// The `kind` string comes from the parsed symbol data so that P4 can
/// prefer kind-compatible candidates over incidental name collisions.
/// True when a parsed file or symbol_id_map key is an external dependency
/// source rather than a project file. Externals are identified by the
/// synthetic `ext:` virtual path prefix used by `indexer::externals`.
#[inline]
pub(super) fn is_external_path(path: &str) -> bool {
    path.starts_with("ext:")
}
