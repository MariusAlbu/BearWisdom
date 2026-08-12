// =============================================================================
// engine/rules/external_by_import — import-scoped bind to an external symbol
//
// Gated on `profile.imports.external_by_import: Option<&ExternalByImport>`.  When the
// profile opts in, a bare target with no dots or `::` is resolved against the
// file's EXTERNAL symbol index, restricted to symbols whose file is reachable
// from one of the file's non-relative imports.
//
// Two matching modes (`profile.imports.ext_match`):
//   PkgSegment   — external file's `ext:<lang>:<pkg>` segment equals an import
//                  root, or starts with `{root}-` (gem family: `aws-sdk-s3`
//                  under `aws`).
//   FileStemOrDir — external file's basename-stem / a dir-segment equals an
//                  import LEAF (last path segment, `std/`/`pkg/` dropped) or an
//                  import PACKAGE (first path segment), checked via
//                  `path_stem_matches`.
// =============================================================================

use crate::indexer::resolve::engine::support::path_stem_matches;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
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
        if target.is_empty() || target.contains('.') || target.contains("::") {
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
                        if m.starts_with('.') {
                            return None;
                        }
                        Some(m.split('/').next().unwrap_or(m).to_string())
                    })
                    .collect();
                if import_roots.is_empty() {
                    return LookupResult::Pass;
                }
                ExtFileMatcher::PkgSegment(import_roots)
            }
            ExtMatch::FileStemOrDir => {
                // Leaf = last path segment (with `std/`/`pkg/` prefix dropped);
                // package = first path segment (skipping `std`/`pkg` roots).
                let mut needles: Vec<String> = Vec::new();
                for imp in &ctx.file_ctx.imports {
                    let Some(m) = imp.module_path.as_deref() else {
                        continue;
                    };
                    if m.starts_with('.') {
                        continue;
                    }
                    let stripped = m
                        .strip_prefix("std/")
                        .or_else(|| m.strip_prefix("pkg/"))
                        .unwrap_or(m);
                    let leaf = stripped.rsplit('/').next().unwrap_or(stripped);
                    if !leaf.is_empty() {
                        needles.push(leaf.to_lowercase());
                    }
                    let pkg = m.split('/').next().unwrap_or(m);
                    if pkg != "std" && pkg != "pkg" && !pkg.is_empty() {
                        needles.push(pkg.to_lowercase());
                    }
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
                ExtFileMatcher::PkgSegment(roots) => {
                    let pkg_seg = external_package_segment(&sym.file_path);
                    !pkg_seg.is_empty()
                        && roots
                            .iter()
                            .any(|root| pkg_seg == root.as_str() || pkg_seg.starts_with(&format!("{root}-")))
                }
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

/// The package segment of an external file path under the
/// `ext:<lang>:<pkg>/…` convention.  For `ext:ruby:aws-sdk-s3/lib/x.rb`
/// returns `aws-sdk-s3`; for paths that don't match the three-colon shape
/// returns `""`.
fn external_package_segment(path: &str) -> &str {
    let Some(rest) = path.strip_prefix("ext:") else {
        return "";
    };
    let Some((_lang, after_lang)) = rest.split_once(':') else {
        return "";
    };
    after_lang.split('/').next().unwrap_or("")
}

#[cfg(test)]
#[path = "external_by_import_tests.rs"]
mod tests;
