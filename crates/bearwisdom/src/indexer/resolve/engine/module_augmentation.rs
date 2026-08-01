// =============================================================================
// engine/module_augmentation — cross-module interface grafts in external source
//
// A package can declare members onto an interface that belongs to ANOTHER
// module (`declare module 'chai' { interface Assertion { … } }`). The members
// land under the augmenting package's own qname, so the compilation needs the
// `(augmented module, interface, augmenting qname)` triples to graft them onto
// the interface they extend. Source-text scanning, because the external parse
// cache discards file content.
// =============================================================================

use std::path::Path;

/// Scan one external file's on-disk source for TS module augmentations,
/// appending `(augmented_module, interface, augmenting_qname)` for each. The
/// augmenting interface's qname is `<package>.<interface>` (post-process
/// prefixes external symbols by package); its supertypes are already in the
/// compilation's inherits map. A package augmenting its own module is skipped —
/// that is ordinary in-package declaration, not a cross-module graft.
pub(crate) fn collect_module_augmentations(
    importer: &Path,
    virtual_path: &str,
    out: &mut Vec<(String, String, String)>,
) {
    let Ok(content) = std::fs::read_to_string(importer) else {
        return;
    };
    if !content.contains("declare module") {
        return;
    }
    let Some(pkg) = crate::ecosystem::externals::ts_package_from_virtual_path(virtual_path) else {
        return;
    };
    for (module, iface) in scan_module_augmentations(&content) {
        if module == pkg {
            continue;
        }
        let aug_qname = format!("{pkg}.{iface}");
        out.push((module, iface, aug_qname));
    }
}

/// Scan TS source for `declare module '<M>' { … interface <I> … }` blocks,
/// returning each `(M, I)` pair. Only a QUOTED module name is an augmentation
/// (`declare module Foo` without quotes is a namespace). Line-oriented with
/// brace depth tracking — the augmentation bodies in `.d.ts` files are flat
/// interface lists, so a depth counter is sufficient to bound each block.
pub(crate) fn scan_module_augmentations(content: &str) -> Vec<(String, String)> {
    use crate::ecosystem::npm::extract_first_quoted;
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut depth: i32 = 0;
    for line in content.lines() {
        let t = line.trim();
        match &current {
            None => {
                if let Some(rest) = t.strip_prefix("declare module ") {
                    if let Some(m) = extract_first_quoted(rest.trim_start()) {
                        current = Some(m.to_string());
                        depth = brace_delta(t);
                        if depth <= 0 {
                            current = None;
                        }
                    }
                }
            }
            Some(module) => {
                if let Some(iface) = interface_name(t) {
                    out.push((module.clone(), iface.to_string()));
                }
                depth += brace_delta(t);
                if depth <= 0 {
                    current = None;
                }
            }
        }
    }
    out
}

/// Net `{` minus `}` count on a line.
fn brace_delta(line: &str) -> i32 {
    line.matches('{').count() as i32 - line.matches('}').count() as i32
}

/// The interface name an `interface <I>` / `export interface <I>` line
/// declares, with any generic parameter list stripped
/// (`Assertion<T = any>` → `Assertion`).
pub(crate) fn interface_name(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("export interface ")
        .or_else(|| line.strip_prefix("interface "))?;
    let name = rest
        .split(|c: char| c == '<' || c == ' ' || c == '{')
        .next()?
        .trim();
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
#[path = "module_augmentation_tests.rs"]
mod tests;
