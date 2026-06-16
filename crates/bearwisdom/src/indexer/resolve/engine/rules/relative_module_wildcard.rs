// =============================================================================
// engine/rules/relative_module_wildcard — Rust relative-module glob bind
//
// A `use super::*` or `use crate::x::y::*` glob import brings every symbol
// DEFINED in the named sibling/ancestor module file into scope.  Unlike the
// qname-prefix wildcard, Rust top-level symbols carry no module-prefixed qname
// (`find_position_of`, not `tests::find_position_of`), so the bind is by
// (file, bare name): compute the candidate file paths the keyword names, then
// match the bare target against symbols defined in those files.
//
// Fires only when a wildcard import's `module_path` is a Rust-relative keyword
// (`super`, `crate`, or a `super::`/`crate::`-rooted path).  Declines a bare
// target with `.` or `::`, on zero module-file candidates, and on two distinct
// id hits (ambiguous glob).
//
// Inlined helpers: `relative_module_files`, `module_parent_dir`, `crate_src_dir`,
// `module_roots`.  All use support::parent_dir.
// =============================================================================

use crate::indexer::resolve::engine::support::parent_dir;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct RelativeModuleWildcardRule;

impl LookupRule for RelativeModuleWildcardRule {
    fn name(&self) -> &'static str {
        "relative_module_wildcard"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();

        // Collect the candidate files named by every relative-module glob.
        let mut module_files: Vec<String> = Vec::new();
        for imp in ctx.file_ctx.imports.iter().filter(|i| i.is_wildcard) {
            let Some(module) = imp.module_path.as_deref() else {
                continue;
            };
            for f in relative_module_files(&ctx.file_ctx.file_path, module) {
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
            Some(id) => LookupResult::Resolved(ctx.resolved(id, "default_relative_module_wildcard")),
            None => LookupResult::Pass,
        }
    }
}

// =============================================================================
// Private helpers
// =============================================================================

/// The candidate root files for a Rust relative-module glob.  Returns an empty
/// vec for a non-relative module (`std`, an external crate name) so the rule
/// stays inert.  Path separators are normalized to `/`.
fn relative_module_files(importing_file: &str, module: &str) -> Vec<String> {
    let file = importing_file.replace('\\', "/");
    let head = module.split("::").next().unwrap_or(module);
    match head {
        "super" => {
            let Some(parent) = module_parent_dir(&file) else {
                return Vec::new();
            };
            let rest: Vec<&str> = module.split("::").skip(1).collect();
            module_roots(&parent, &rest)
        }
        "crate" => {
            let src_dir = crate_src_dir(&file);
            let rest: Vec<&str> = module.split("::").skip(1).collect();
            if rest.is_empty() {
                vec![format!("{src_dir}/lib.rs"), format!("{src_dir}/main.rs")]
            } else {
                module_roots(&src_dir, &rest)
            }
        }
        _ => Vec::new(),
    }
}

/// The directory of the importing file's PARENT module.  For a regular
/// `D/foo.rs` the parent module directory is `D`.  For a `D/mod.rs` /
/// `D/lib.rs` / `D/main.rs` the file IS module `D`, so the parent is
/// `parent(D)`.  Returns `None` when no parent directory exists.
fn module_parent_dir(file: &str) -> Option<String> {
    let dir = parent_dir(file)?;
    let basename = file.rsplit('/').next().unwrap_or(file);
    let stem = basename.rsplit_once('.').map_or(basename, |(s, _)| s);
    if matches!(stem, "mod" | "lib" | "main") {
        parent_dir(&dir)
    } else {
        Some(dir)
    }
}

/// The crate's source-root directory: the prefix up to and including the last
/// `src/` segment of `file`.  Falls back to the importing file's own directory
/// when the path has no `src/` segment.
fn crate_src_dir(file: &str) -> String {
    let mut acc: Vec<&str> = Vec::new();
    let mut last_src: Option<usize> = None;
    for (i, seg) in file.split('/').enumerate() {
        acc.push(seg);
        if seg == "src" {
            last_src = Some(i);
        }
    }
    match last_src {
        Some(i) => acc[..=i].join("/"),
        None => parent_dir(file).unwrap_or_default(),
    }
}

/// The candidate root files for a module reached by descending `segments` from
/// `base_dir`.  With no segments, yields `<base_dir>/mod.rs` and the sibling
/// `<base_dir>.rs`.  With segments, yields the file form and the directory form.
fn module_roots(base_dir: &str, segments: &[&str]) -> Vec<String> {
    if segments.is_empty() {
        let leaf = base_dir.rsplit('/').next().unwrap_or(base_dir);
        let sibling = parent_dir(base_dir)
            .map(|p| format!("{p}/{leaf}.rs"))
            .unwrap_or_else(|| format!("{base_dir}.rs"));
        return vec![format!("{base_dir}/mod.rs"), sibling];
    }
    let nested = segments.join("/");
    vec![
        format!("{base_dir}/{nested}.rs"),
        format!("{base_dir}/{nested}/mod.rs"),
    ]
}

#[cfg(test)]
#[path = "relative_module_wildcard_tests.rs"]
mod tests;
