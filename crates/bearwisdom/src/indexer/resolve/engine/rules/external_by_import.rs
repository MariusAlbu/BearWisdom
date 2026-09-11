// =============================================================================
// engine/rules/external_by_import — import-scoped bind to an external symbol
//
// Gated on `profile.imports.external_by_import: Option<&ExternalByImport>`.  When the
// profile opts in, a bare target with no profile qname separator is resolved against the
// file's EXTERNAL symbol index, restricted to symbols whose file is reachable
// from one of the file's non-relative imports.
//
// Two matching modes (`profile.imports.ext_match`):
//   PkgSegment   — adapter-owned package identity from an external file matches
//                  an import root.
//   FileStemOrDir — external file's basename-stem / a dir-segment equals one
//                  of the profile adapter's import-path terms, checked via
//                  `path_stem_matches`.
// =============================================================================

use crate::indexer::resolve::engine::support::path_stem_matches;
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::ExtMatch;

pub struct ExternalByImportRule;

impl LookupRule for ExternalByImportRule {
    fn name(&self) -> &'static str {
        "external_by_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        // Gated — opt-in per language.
        if ctx.profile.imports.external_by_import.is_none() {
            return LookupResult::Pass;
        }

        let target = ctx.target();
        if target.is_empty() || ctx.profile.is_qualified_name(target) {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let ext_match = ctx.profile.imports.ext_match;

        let matcher = match ext_match {
            ExtMatch::PkgSegment => {
                let import_roots: Vec<String> = ctx
                    .file_ctx
                    .imports
                    .iter()
                    .filter_map(|imp| {
                        let m = imp.module_path.as_deref()?;
                        if ctx.profile.source_module_path_policy(m).is_relative(m) {
                            return None;
                        }
                        crate::ecosystem::package_specifier::import_package_root(
                            &ctx.file_ctx.language,
                            m,
                        )
                    })
                    .collect();
                if import_roots.is_empty() {
                    return LookupResult::Pass;
                }
                ExtFileMatcher::PkgSegment(import_roots)
            }
            ExtMatch::FileStemOrDir => {
                let mut needles: Vec<String> = Vec::new();
                for imp in &ctx.file_ctx.imports {
                    let Some(m) = imp.module_path.as_deref() else {
                        continue;
                    };
                    if ctx.profile.source_module_path_policy(m).is_relative(m) {
                        continue;
                    }
                    needles.extend((ctx
                        .profile
                        .source_module_path_policy(m)
                        .external_import_match_terms)(m));
                }
                if needles.is_empty() {
                    return LookupResult::Pass;
                }
                ExtFileMatcher::FileStemOrDir(needles)
            }
        };

        for sym in ctx.lookup.by_name(target) {
            if !ctx.lookup.is_external_file(&sym.file_path) {
                continue;
            }
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            let matched = match &matcher {
                ExtFileMatcher::PkgSegment(roots) => roots.iter().any(|root| {
                    crate::ecosystem::package_specifier::external_package_matches_import(
                        &ctx.file_ctx.language,
                        &sym.file_path,
                        root,
                    )
                    .unwrap_or(false)
                }),
                ExtFileMatcher::FileStemOrDir(needles) => {
                    let file_lower = sym.file_path.to_lowercase();
                    needles.iter().any(|n| path_stem_matches(&file_lower, n))
                }
            };
            if matched {
                return LookupResult::Resolved(ctx.resolved(sym.id, "default_external_by_import"));
            }
        }
        LookupResult::Pass
    }
}

// =============================================================================
// Private helpers — used only by this rule
// =============================================================================

/// Pre-computed matcher built from the import set, used to avoid re-scanning
/// imports for every candidate symbol.
enum ExtFileMatcher {
    PkgSegment(Vec<String>),
    FileStemOrDir(Vec<String>),
}

#[cfg(test)]
#[path = "external_by_import_tests.rs"]
mod tests;
