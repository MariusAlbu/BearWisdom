// =============================================================================
// typescript/angular_module_reachables.rs — the declaration files an Angular
// NgModule `.d.ts` reaches
// =============================================================================

/// An Angular NgModule declaration `.d.ts` reaches the `.component`/`.directive`
/// `.d.ts` files it declares — components/directives are referenced only by
/// selector, so nothing demands them by name; descending the module's
/// declarations is the structural signal that materializes them (and their
/// `ɵcmp`/`ɵdir` selectors). Gated on the `ɵɵNgModuleDeclaration` marker, so
/// non-Angular `.d.ts` cost nothing.
pub(super) fn reachables(file_path: &str, content: &str) -> Vec<String> {
    if !file_path.ends_with(".module.d.ts") || !content.contains("ɵɵNgModuleDeclaration") {
        return Vec::new();
    }
    let mut out = Vec::new();
    for line in content.lines() {
        let t = line.trim();
        if !(t.starts_with("import ") || t.starts_with("export ")) {
            continue;
        }
        let Some(spec) = crate::ecosystem::npm::extract_quoted_after(t, " from ") else {
            continue;
        };
        if spec.starts_with('.') && (spec.contains(".component") || spec.contains(".directive")) {
            out.push(spec.to_string());
        }
    }
    out
}
