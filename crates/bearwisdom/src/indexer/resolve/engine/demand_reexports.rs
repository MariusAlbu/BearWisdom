// =============================================================================
// engine/demand_reexports — next-frontier files a materialized file re-exports
//
// A package entry rarely declares its surface; it forwards it through relative
// re-export hops (`export * from './dist'`, `export { X } from './x'`). The
// module graph binds an import only by walking those hops module by module,
// and a hop whose file was never materialized makes every wildcard behind it
// incomplete. Pulling the leaf that declares a name is therefore not enough:
// each relative re-export a pulled file names is itself the next frontier,
// exactly as a compiler loads every module a declaration file forwards to.
// =============================================================================

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::ecosystem::external_policy;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::ExtractedRef;

/// Collect the files named by `refs`' RELATIVE re-exports into `out`, resolved
/// against `abs`'s directory through the language's owning ecosystem. Bare
/// (package) re-exports are the import collector's concern; a language whose
/// ecosystem resolves no relative modules contributes nothing.
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
    for spec in relative_reexport_specs(profile, refs) {
        let Some(file) = external_policy::resolve_relative_module(language, dir, spec) else {
            continue;
        };
        if seen.insert(file.clone()) {
            out.push(file);
        }
    }
}

/// The distinct relative module specifiers `refs` re-export from, in first
/// occurrence order.
fn relative_reexport_specs<'a>(
    profile: &LanguageProfile,
    refs: &'a [ExtractedRef],
) -> Vec<&'a str> {
    let mut specs: Vec<&str> = Vec::new();
    for r in refs.iter().filter(|r| r.is_reexport) {
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
#[path = "demand_reexports_tests.rs"]
mod tests;
