// =============================================================================
// engine/demand_relative_hops — next-frontier files a materialized file names
// by a relative specifier
//
// A package entry rarely declares its surface: it forwards it through
// relative re-exports (`export * from './dist'`, `export { X } from './x'`),
// imports the declarations its own signatures mention, and loads augmentation
// carriers through side-effect imports (`import './chunks/global.js'`). The
// module graph binds through those hops module by module, and a hop whose file
// was never materialized makes every wildcard behind it incomplete. Each
// relative specifier a pulled file names is therefore itself the next
// frontier, exactly as a compiler loads every module a declaration file names.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ecosystem::external_policy;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{EdgeKind, ExtractedRef};

/// Collect the files named by `refs`' RELATIVE imports and re-exports into
/// `out`, resolved against `abs`'s directory through the language's owning
/// ecosystem. Bare (package) specifiers are the import collector's concern; a
/// language whose ecosystem resolves no relative modules contributes nothing.
pub(super) fn collect(
    abs: &Path,
    language: &str,
    profile: &LanguageProfile,
    refs: &[ExtractedRef],
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let Some(dir) = abs.parent() else {
        return;
    };
    for spec in relative_specs(profile, refs) {
        let Some(file) = external_policy::resolve_relative_module(language, dir, spec) else {
            continue;
        };
        if seen.insert(file.clone()) {
            out.push(file);
        }
    }
}

/// The distinct relative module specifiers `refs` import or re-export from,
/// in first occurrence order.
fn relative_specs<'a>(profile: &LanguageProfile, refs: &'a [ExtractedRef]) -> Vec<&'a str> {
    let mut specs: Vec<&str> = Vec::new();
    for r in refs
        .iter()
        .filter(|r| r.is_reexport || r.is_import_binding || r.kind == EdgeKind::Imports)
    {
        let Some(spec) = r.module.as_deref() else {
            continue;
        };
        if profile.module_anchor_is_relative(spec) && !specs.contains(&spec) {
            specs.push(spec);
        }
    }
    specs
}

#[cfg(test)]
#[path = "demand_relative_hops_tests.rs"]
mod tests;
