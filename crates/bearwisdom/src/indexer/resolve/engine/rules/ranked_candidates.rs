// =============================================================================
// engine/rules/ranked_candidates — proximity-scored tiebreaker for bare names
//
// Gated on `ctx.profile.multi_candidate_ranking` (false → Pass). Fires only
// when `by_name` yields two or more kind-compatible candidates for a bare
// (separator-free) target — single-candidate refs are left to the earlier
// strict-name rules. Scores each candidate by workspace-package membership,
// import-module proximity, ambient status, path proximity, external-path depth
// penalty, and visibility, then accepts the top scorer only when it beats the
// runner-up by at least `RANK_MARGIN`. A margin smaller than that leaves the
// ref honestly unresolved rather than guessing.
// =============================================================================

use crate::indexer::resolve::engine::support::qname_under_module;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::indexer::resolve::engine::contract::Symbol;

pub struct RankedCandidatesRule;

/// Minimum score margin the top candidate must beat the runner-up by.
const RANK_MARGIN: i32 = 100;

impl LookupRule for RankedCandidatesRule {
    fn name(&self) -> &'static str {
        "ranked_candidates"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if !ctx.profile.multi_candidate_ranking {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty()
            || target.contains('.')
            || target.contains("::")
            || target.contains('/')
        {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let candidates: Vec<&Symbol> = ctx
            .lookup
            .by_name(target)
            .into_iter()
            .filter(|sym| (ctx.kind)(edge_kind, &sym.kind))
            .collect();
        if candidates.len() < 2 {
            // Zero or one: the earlier strict rules had their chance. Stay out.
            return LookupResult::Pass;
        }
        let mut scored: Vec<(i32, &Symbol)> = candidates
            .iter()
            .map(|sym| (score_candidate(ctx, sym), *sym))
            .collect();
        // Highest score first; tie-break by id for determinism.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
        let (top_score, top) = scored[0];
        let (runner_score, _) = scored[1];
        if top_score - runner_score < RANK_MARGIN {
            return LookupResult::Pass;
        }
        LookupResult::Resolved(ctx.resolved(top.id, "default_ranked_candidate"))
    }
}

/// Score a candidate against the ref context. Higher = better. Stateless:
/// reads only `ctx` and the candidate's index row.
fn score_candidate(ctx: &BinderContext<'_>, sym: &Symbol) -> i32 {
    let mut s: i32 = 0;

    // Same workspace package as caller.
    if let (Some(caller_pkg), Some(sym_pkg)) =
        (ctx.ref_ctx.file_package_id, sym.package_id)
    {
        if caller_pkg == sym_pkg {
            s += 1000;
        }
    }

    // Caller has an import that points at this candidate's package or namespace.
    let mut matched_workspace_import = false;
    for import in &ctx.file_ctx.imports {
        let Some(mod_path) = import.module_path.as_deref() else {
            continue;
        };
        if let Some(wp_id) = ctx.lookup.workspace_package_id(mod_path) {
            if Some(wp_id) == sym.package_id {
                s += 500;
                matched_workspace_import = true;
            }
        }
        if qname_under_module(&sym.qualified_name, mod_path) {
            s += 300;
        }
    }
    let _ = matched_workspace_import;

    // Ambient candidates are project-declared in-scope providers.
    if ctx.lookup.is_ambient_path(&sym.file_path) {
        s += 200;
    }

    // File-path proximity: +10 per shared leading directory segment.
    s += path_proximity_score(&ctx.file_ctx.file_path, &sym.file_path);

    // External candidates take a small depth penalty.
    if ctx.lookup.is_external_file(&sym.file_path) {
        let depth = sym.file_path.matches('/').count() as i32;
        s -= depth.min(20);
    }

    // Visibility hint.
    match sym.visibility.as_deref() {
        Some("public") => s += 50,
        Some("private") => s -= 200,
        _ => {}
    }

    s
}

/// Shared-directory-prefix score between two file paths. Returns 10 ×
/// number of shared leading directory segments. Path separators are
/// normalised to `/`. The caller's filename is dropped before comparing.
fn path_proximity_score(caller_path: &str, candidate_path: &str) -> i32 {
    let caller_norm = caller_path.replace('\\', "/");
    let candidate_norm = candidate_path.replace('\\', "/");
    let caller_dir = caller_norm.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let candidate_dir = candidate_norm
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("");
    let caller_segs: Vec<&str> = caller_dir.split('/').filter(|s| !s.is_empty()).collect();
    let candidate_segs: Vec<&str> =
        candidate_dir.split('/').filter(|s| !s.is_empty()).collect();
    let shared = caller_segs
        .iter()
        .zip(candidate_segs.iter())
        .take_while(|(a, b)| a == b)
        .count() as i32;
    shared * 10
}

#[cfg(test)]
#[path = "ranked_candidates_tests.rs"]
mod tests;
