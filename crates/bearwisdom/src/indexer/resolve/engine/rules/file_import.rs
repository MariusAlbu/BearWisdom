// =============================================================================
// engine/rules/file_import — bare target imported by name from a module
//
// `import { Foo } from './foo'` then `Foo(...)`: the import entry brings `Foo`
// into local scope. Walk the file's imports; accept any entry that names `target`
// (directly or via an `alias`) and find a same-named symbol whose file path
// matches the import's module specifier.
//
// `file_path_matches_module` is the canonical multi-form matcher: stem suffix,
// dot-to-slash, and segment-bounded package-directory run. It is inlined here
// because it is non-trivial and specific to this rule.
// =============================================================================

use crate::indexer::resolve::engine::support::trim_source_extension;
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

/// Match a symbol's file path against an import module specifier. Returns `true`
/// when the path plausibly names the same file as `module`:
/// - Stem-suffix: the path's extension-stripped form ends with the module's
///   extension-stripped, `./`/`../`-trimmed form (covers relative imports).
/// - Dot-to-slash: same after replacing `.` with `/` in the module (covers
///   dotted package imports like `posthog.models`).
/// - Segment-bounded run: the slash-form of the module appears as a
///   `/`-bounded contiguous run inside the path (covers `__init__.py` re-exports
///   and deep package paths that the stem-suffix check misses).
fn file_path_matches_module(file_path: &str, module: &str) -> bool {
    if module.is_empty() {
        return false;
    }
    let normalized = file_path.replace('\\', "/");
    let cleaned =
        trim_source_extension(module.trim_start_matches("./").trim_start_matches("../"));
    let stem = trim_source_extension(&normalized);
    if stem.ends_with(cleaned) || stem.ends_with(&cleaned.replace('.', "/")) {
        return true;
    }
    let dotted = cleaned.replace('.', "/");
    if dotted.is_empty() {
        return false;
    }
    path_contains_segment_run(&normalized, &dotted)
}

/// `true` when `run` appears in `path` as a `/`-bounded contiguous segment run.
fn path_contains_segment_run(path: &str, run: &str) -> bool {
    let mut from = 0;
    while let Some(rel) = path[from..].find(run) {
        let start = from + rel;
        let end = start + run.len();
        let left_ok = start == 0 || path.as_bytes()[start - 1] == b'/';
        let right_ok = end == path.len() || path.as_bytes()[end] == b'/';
        if left_ok && right_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

#[cfg(test)]
#[path = "file_import_tests.rs"]
mod tests;
