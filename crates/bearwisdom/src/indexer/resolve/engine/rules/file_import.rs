// =============================================================================
// engine/rules/file_import — bare target imported by name from a module
//
// `import { Foo } from './foo'` then `Foo(...)`: the import entry brings `Foo`
// into local scope. Walk the file's imports; accept any entry that names `target`
// (directly or via an `alias`) and find a same-named symbol whose file path
// matches the import's module specifier.
//
// `file_path_matches_module` (engine/path_match) is the canonical multi-form
// matcher: stem suffix, dot-to-slash, and segment-bounded package-directory run.
// =============================================================================

use crate::indexer::resolve::engine::support::file_path_matches_module;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct FileImportRule;

impl LookupRule for FileImportRule {
    fn name(&self) -> &'static str {
        "file_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();

        for import in &ctx.file_ctx.imports {
            let matches_direct = import.imported_name == target;
            let matches_alias = import.alias.as_deref() == Some(target);
            if !matches_direct && !matches_alias {
                continue;
            }
            let lookup_name = if matches_alias {
                &import.imported_name
            } else {
                target
            };
            let module_path = import.module_path.as_deref().unwrap_or("");

            for sym in ctx.lookup.by_name(lookup_name) {
                if !(ctx.kind)(edge_kind, &sym.kind) {
                    continue;
                }
                if file_path_matches_module(&sym.file_path, module_path) {
                    return LookupResult::Resolved(ctx.resolved(sym.id, "default_file_import"));
                }
            }
        }
        LookupResult::Pass
    }
}


#[cfg(test)]
#[path = "file_import_tests.rs"]
mod tests;
