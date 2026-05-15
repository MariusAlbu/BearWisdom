// =============================================================================
// indexer/resolve/engine/util.rs — pure utility functions for the resolve layer
//
// Small predicates and helpers with no dependency on SymbolIndex or the
// resolve loop's state: scope-chain construction from a qualified name,
// ambient-global detection for `lib.dom.d.ts` / `@types/node` paths,
// type-kind classification, npm-package extraction from external paths and
// import specifiers.
// =============================================================================


/// Detect an ambient-global TypeScript declaration file — `lib.*.d.ts` shipped
/// with the TypeScript compiler, or any file under `@types/node/`. Methods
/// declared in these files are the JS/DOM/ES runtime surface and need no
/// explicit import to call.
///
/// Recognises both the historical absolute-path form
/// (`.../typescript/lib/lib.dom.d.ts`) and the synthetic-module form
/// emitted post-Pass-A (`ext:ts:__ts_lib__/lib.dom.d.ts`,
/// `ext:ts:@types/node/process.d.ts`). The substring matchers stay so
/// older indexes built before the path rewrite still classify correctly.
pub(crate) fn is_ambient_global_lib_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let synthetic_prefix = format!(
        "ext:ts:{}/",
        crate::ecosystem::ts_lib_dom::TS_LIB_SYNTHETIC_MODULE
    );
    normalized.starts_with(&synthetic_prefix)
        || normalized.starts_with("ext:ts:@types/node/")
        || normalized.contains("/typescript/lib/lib.")
        || normalized.contains("/@types/node/")
}

pub(crate) fn is_type_like_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "struct"
            | "interface"
            | "enum"
            | "type_alias"
            | "namespace"
            | "record"
            | "trait"
            | "protocol"
            | "object"
            | "mixin"
            | "extension"
    )
}

pub(crate) fn common_prefix_len(a: &str, b: &str) -> usize {
    a.split('.').zip(b.split('.')).take_while(|(x, y)| x == y).count()
}

// ---------------------------------------------------------------------------
// Helpers for building RefContext
// ---------------------------------------------------------------------------

/// Build the scope chain from a symbol's scope_path.
///
/// scope_path = "A.B.C" → ["A.B.C", "A.B", "A"]
pub fn build_scope_chain(scope_path: Option<&str>) -> Vec<String> {
    let Some(path) = scope_path else {
        return Vec::new();
    };
    if path.is_empty() {
        return Vec::new();
    }

    let mut chain = Vec::new();
    let mut current = path.to_string();
    chain.push(current.clone());

    while let Some(dot_pos) = current.rfind('.') {
        current.truncate(dot_pos);
        chain.push(current.clone());
    }

    chain
}

/// Extract the npm package name from an external file path.
///
/// External paths from the TS externals walker look like:
///   `ext:ts:@scope/pkg/dist/whatever.d.ts` → `@scope/pkg`
///   `ext:ts:lodash/index.d.ts`             → `lodash`
///
/// Returns None when the path isn't an `ext:ts:` external or doesn't have
/// a recognisable package prefix. Used by the cross-package re-export
/// chain resolver to bucket files by their owning npm package.
pub fn npm_package_from_external_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("ext:ts:")?;
    if rest.starts_with('@') {
        let mut parts = rest.splitn(3, '/');
        match (parts.next(), parts.next()) {
            (Some(scope), Some(name)) if !scope.is_empty() && !name.is_empty() => {
                Some(format!("{scope}/{name}"))
            }
            _ => None,
        }
    } else {
        let pkg = rest.split('/').next().unwrap_or("");
        if pkg.is_empty() { None } else { Some(pkg.to_string()) }
    }
}

/// Extract the npm package name from an import / re-export specifier.
///
/// Specifiers can carry sub-paths: `@scope/pkg/sub`, `lodash/fp`. We keep
/// only the first segment (or first two for scoped packages). Returns
/// None for relative specifiers (`./x`, `../y`) since those don't cross
/// package boundaries.
pub fn npm_package_from_specifier(spec: &str) -> Option<String> {
    if spec.starts_with("./") || spec.starts_with("../") || spec.is_empty() {
        return None;
    }
    if spec.starts_with('@') {
        let mut parts = spec.splitn(3, '/');
        match (parts.next(), parts.next()) {
            (Some(scope), Some(name)) if !scope.is_empty() && !name.is_empty() => {
                Some(format!("{scope}/{name}"))
            }
            _ => None,
        }
    } else {
        let pkg = spec.split('/').next().unwrap_or("");
        if pkg.is_empty() { None } else { Some(pkg.to_string()) }
    }
}

/// Test whether `file_path` belongs to the given npm package (`pkg`).
/// Matches the synthetic `ext:ts:<pkg>/...` prefix as well as raw
/// `node_modules/<pkg>/...` substring (for files that landed via
/// non-prefixed paths in older indexes).
pub fn file_belongs_to_npm_package(file_path: &str, pkg: &str) -> bool {
    let needle_ext = format!("ext:ts:{pkg}/");
    if file_path.starts_with(&needle_ext) {
        return true;
    }
    let needle_nm = format!("node_modules/{pkg}/");
    file_path.contains(&needle_nm)
}


#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
