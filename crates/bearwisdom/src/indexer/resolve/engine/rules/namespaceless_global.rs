// =============================================================================
// engine/rules/namespaceless_global — first-match bind in a flat global
// namespace
//
// For a language with no import, namespace, or scope structure (SQL and other
// namespaceless DDL/config languages), a bare `target` binds to the first
// kind-compatible project symbol of the same name. A supplied declaration is
// eligible only when the lookup proves the source reaches its file through a
// transitive C/C++ include chain.
//
// `NamespaceScope` gates and shapes the rung:
//   - `Off`             — returns Pass immediately (every non-flat language).
//   - `Global`          — first-match-bind across the whole project.
//   - `DirectoryScoped` — binds only a candidate in the same directory as
//                         the referencing file.
//
// Name candidates in order: the raw target; the self-keyword-stripped leaf
// (flat-namespace sigil languages keep the sigil in the ref — `var.X` — but
// declare the symbol bare); then, for a dotted target that missed both, its
// last `.`-segment (`schema.table` → `table`).
//
// Runs last in the ladder so every structural rung above wins first.
// =============================================================================

use crate::indexer::resolve::engine::support::{parent_dir, strip_self_keyword};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::NamespaceScope;

pub struct NamespacelessGlobalRule;

impl LookupRule for NamespacelessGlobalRule {
    fn name(&self) -> &'static str {
        "namespaceless_global"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let scope = ctx.profile.namespaceless_global_type_lookup;
        if scope == NamespaceScope::Off {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let self_keywords = ctx.profile.self_keywords;
        let stripped = strip_self_keyword(target, self_keywords);
        let mut candidates: Vec<&str> = vec![target];
        if stripped != target {
            candidates.push(stripped);
        }
        // Dotted-target leaf fallback (`schema.table` → `table`): only the last
        // `.`-segment, only when distinct from what's already queued.
        if let Some(leaf) = target.rsplit('.').next() {
            if leaf != target && !candidates.contains(&leaf) {
                candidates.push(leaf);
            }
        }
        let dir_scoped = scope == NamespaceScope::DirectoryScoped;
        let src_dir = parent_dir(&ctx.file_ctx.file_path);
        for cand in candidates {
            // Project declarations keep the historical first-match behavior
            // and always take precedence over supplied headers.
            for sym in ctx.lookup.by_name(cand) {
                if ctx.lookup.is_external_file(&sym.file_path) {
                    continue;
                }
                // DirectoryScoped: bind only a candidate in the same directory as
                // the referencing file (both `None` — repo-root files — match).
                if dir_scoped && parent_dir(&sym.file_path) != src_dir {
                    continue;
                }
                if (ctx.kind)(edge_kind, &sym.kind) {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "default_namespaceless_global"),
                    );
                }
            }

            // Header visibility is file-identity evidence. Require one unique
            // compatible declaration; two reachable external declarations are
            // ambiguous and leave the reference unresolved.
            let mut included = ctx
                .lookup
                .by_name(cand)
                .into_iter()
                .filter(|sym| ctx.lookup.is_external_file(&sym.file_path))
                .filter(|sym| {
                    ctx.lookup
                        .include_reaches(&ctx.file_ctx.file_path, &sym.file_path)
                })
                .filter(|sym| (ctx.kind)(edge_kind, &sym.kind));
            let Some(first) = included.next() else {
                continue;
            };
            if included.any(|sym| sym.id != first.id) {
                continue;
            }
            return LookupResult::Resolved(
                ctx.resolved(first.id, "include_reachable_namespaceless_global"),
            );
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "namespaceless_global_tests.rs"]
mod tests;
