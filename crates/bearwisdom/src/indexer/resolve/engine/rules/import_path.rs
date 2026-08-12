// =============================================================================
// engine/rules/import_path — template include / file-import path bind
//
// Languages whose `Imports` refs name another file by relative path or stem
// (handlebars partials, EJS / Pug includes, GSP renders, markdown relative
// links, YAML `uses`) resolve via this rule.  The profile's `import_resolution`
// field carries the full candidate-generation spec; `None` leaves the rule inert.
//
// The rule fires ONLY on `EdgeKind::Imports` refs (the predicate is hard-wired
// to the edge kind, not delegated to the kind predicate — the bind_kind check
// is already part of the candidate scan).
//
// `import_path_candidates` is inlined here.  It uses `camel_to_kebab` and
// `lexical_normalize` from the engine's `contract` module.
// =============================================================================

use crate::indexer::resolve::engine::contract::{camel_to_kebab, lexical_normalize};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{CandidateDirs, ImportResolution, StemMatch};
use crate::types::EdgeKind;

pub struct ImportPathRule;

impl LookupRule for ImportPathRule {
    fn name(&self) -> &'static str {
        "import_path"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let Some(ir) = ctx.profile.imports.import_resolution.as_ref() else {
            return LookupResult::Pass;
        };
        if ctx.edge_kind() != EdgeKind::Imports {
            return LookupResult::Pass;
        }
        let target = ctx.r().target_name.trim();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        if ir.decline_leading_slash && target.starts_with('/') {
            return LookupResult::Pass;
        }
        let Some(source_dir) =
            std::path::Path::new(ctx.file_ctx.file_path.as_str()).parent()
        else {
            return LookupResult::Pass;
        };

        for candidate in import_path_candidates(source_dir, target, ir) {
            let path_str = candidate.to_string_lossy().replace('\\', "/");
            let file_stem = candidate
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let file_name = candidate
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            for sym in ctx.lookup.in_file(&path_str) {
                if sym.kind != ir.bind_kind {
                    continue;
                }
                let name_ok = match ir.stem_match {
                    StemMatch::StemExact => sym.name == file_stem,
                    StemMatch::StemOrUnderscoreStripped => {
                        sym.name == file_stem
                            || sym.name == file_stem.trim_start_matches('_')
                    }
                    StemMatch::BasenameWithExt => sym.name == file_name,
                    StemMatch::AnyClassInFile => true,
                };
                if name_ok {
                    return LookupResult::Resolved(ctx.resolved(sym.id, ir.strategy_tag));
                }
            }
        }
        LookupResult::Pass
    }
}

// =============================================================================
// Private helpers
// =============================================================================

/// Generate the ordered candidate file paths for one import target.  The
/// source-dir-joined base is always first; when `CandidateDirs::WalkUp` is
/// configured, additional parent-directory entries follow.
fn import_path_candidates(
    source_dir: &std::path::Path,
    target: &str,
    ir: &ImportResolution,
) -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut out: Vec<PathBuf> = Vec::with_capacity(32);

    // Append the full base candidate set for one base path.  `base` is the
    // source_dir-joined, lexically-normalized variant path.
    let push_base_set = |out: &mut Vec<PathBuf>, base: PathBuf| {
        let already_ext = base
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| ir.extensions.contains(&e))
            .unwrap_or(false);
        out.push(base.clone());
        if !already_ext {
            let base_str = base.to_string_lossy().to_string();
            for ext in ir.extensions {
                out.push(PathBuf::from(format!("{base_str}.{ext}")));
            }
            for entry in ir.index_files {
                for ext in ir.extensions {
                    out.push(base.join(format!("{entry}.{ext}")));
                }
            }
        }
        if ir.underscore_variant {
            if let (Some(parent), Some(stem)) =
                (base.parent(), base.file_name().and_then(|n| n.to_str()))
            {
                let underscored = parent.join(format!("_{stem}"));
                out.push(underscored.clone());
                if !already_ext {
                    let und_str = underscored.to_string_lossy().to_string();
                    for ext in ir.extensions {
                        out.push(PathBuf::from(format!("{und_str}.{ext}")));
                    }
                }
            }
        }
    };

    let mut name_variants: Vec<String> = vec![target.to_string()];
    if ir.kebab_variant {
        if let Some(kebab) = camel_to_kebab(target) {
            name_variants.push(kebab);
        }
    }

    for variant in &name_variants {
        let direct = lexical_normalize(&source_dir.join(variant));
        push_base_set(&mut out, direct);

        if let CandidateDirs::WalkUp { dirs, depth } = ir.candidate_dirs {
            let mut current = Some(source_dir);
            let mut level = 0usize;
            while let Some(dir) = current {
                for d in dirs {
                    let base = lexical_normalize(&dir.join(d).join(variant));
                    push_base_set(&mut out, base);
                }
                level += 1;
                if level > depth {
                    break;
                }
                current = dir.parent();
            }
        }
    }

    out
}

#[cfg(test)]
#[path = "import_path_tests.rs"]
mod tests;
