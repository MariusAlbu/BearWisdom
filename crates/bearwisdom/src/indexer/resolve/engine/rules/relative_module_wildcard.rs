// =============================================================================
// engine/rules/relative_module_wildcard — physical-file wildcard bind
//
// A profile may provide physical-file candidates for a wildcard module. The
// resolver compares those neutral file paths with same-named symbols and
// accepts exactly one id. Source keywords, separators, source-root discovery,
// and module-file conventions remain in the language adapter.
// =============================================================================

use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::WildcardMatch;

pub struct RelativeModuleWildcardRule;

impl LookupRule for RelativeModuleWildcardRule {
    fn name(&self) -> &'static str {
        "relative_module_wildcard"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let WildcardMatch::QnameUnderWithPhysicalFiles { candidate_files } =
            ctx.profile.imports.wildcard_match
        else {
            return LookupResult::Pass;
        };
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();

        // Collect language-supplied candidate files from every wildcard import.
        let mut module_files: Vec<String> = Vec::new();
        for imp in ctx.file_ctx.imports.iter().filter(|i| i.is_wildcard) {
            let Some(module) = imp.module_path.as_deref() else {
                continue;
            };
            for f in candidate_files(&ctx.file_ctx.file_path, module, target) {
                if !module_files.contains(&f) {
                    module_files.push(f);
                }
            }
        }
        if module_files.is_empty() {
            return LookupResult::Pass;
        }

        // Match by (file, bare name).  Two distinct ids → ambiguous glob → Pass.
        let mut hit: Option<i64> = None;
        for sym in ctx.lookup.by_name(target) {
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            let sym_file = sym.file_path.replace('\\', "/");
            if !module_files.iter().any(|f| *f == sym_file) {
                continue;
            }
            match hit {
                None => hit = Some(sym.id),
                Some(h) if h == sym.id => {}
                Some(_) => return LookupResult::Pass,
            }
        }
        match hit {
            Some(id) => {
                LookupResult::Resolved(ctx.resolved(id, "default_relative_module_wildcard"))
            }
            None => LookupResult::Pass,
        }
    }
}

#[cfg(test)]
#[path = "relative_module_wildcard_tests.rs"]
mod tests;
