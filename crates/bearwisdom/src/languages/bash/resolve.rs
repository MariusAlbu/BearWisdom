// =============================================================================
// bash/resolve.rs — Bash resolution rules
//
// Scope rules for Bash/Shell:
//
//   1. Shell source resolution (0.90): `source ./foo.sh` / `. $VAR/bar.sh`
//      brings ALL symbols from the target file into scope.  Fires before the
//      generic import pass because shell's import model is wildcard-by-file,
//      not by-name, so the standard import check never matches.
//   2. Scope chain walk: innermost function → outermost.
//   3. Same-file resolution: all top-level functions are visible within
//      the file.
//
// Bash import model:
//   `source ./lib/foo.sh`           → module = "./lib/foo.sh",  target_name = "foo"
//   `. "$OSH/themes/bar.sh"`        → module = "$OSH/themes/bar.sh", target_name = "bar"
//
// The extractor emits EdgeKind::Imports with target_name = stem and
// module = the raw path string (preserved for suffix matching here).
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

// ---------------------------------------------------------------------------
// Shell path helpers — used by both the resolver and tests
// ---------------------------------------------------------------------------

/// Strip a leading `$VAR` or `${VAR}` prefix from a shell path string,
/// returning the matchable suffix (without the leading slash).
///
/// Examples:
///   `$OSH/themes/foo.sh`     → `"themes/foo.sh"`
///   `${OSH}/themes/foo.sh`   → `"themes/foo.sh"`
///   `./lib/helpers.sh`       → `"lib/helpers.sh"`
///   `../shared/util.sh`      → `"shared/util.sh"`  (best-effort)
///   `/etc/profile`           → `""` (absolute — out of project)
///   `foo.sh`                 → `"foo.sh"` (bare name)
///   `$VAR`                   → `""` (bare variable, no path component)
pub(super) fn shell_path_suffix(raw: &str) -> &str {
    if raw.starts_with('/') {
        // Absolute path — cannot resolve within the project.
        return "";
    }

    // Strip leading `$VAR/` or `${VAR}/`.
    if raw.starts_with('$') {
        // Find the first `/` after the variable reference.
        if let Some(slash) = raw.find('/') {
            return &raw[slash + 1..];
        }
        // Bare `$VAR` with no path component — skip.
        return "";
    }

    // Strip `./` or `../` prefixes (keep the rest).
    raw.trim_start_matches("./").trim_start_matches("../")
}

/// True when `file_path` ends with `suffix` at a path-component boundary.
///
/// ```text
/// file_path = "themes/powerline/powerline.base.sh"
/// suffix    = "powerline/powerline.base.sh"  → true
///
/// file_path = "src/foobar.sh", suffix = "bar.sh"  → false (not a boundary)
/// ```
pub(super) fn ends_with_path_suffix(file_path: &str, suffix: &str) -> bool {
    if file_path == suffix {
        return true;
    }
    if file_path.ends_with(suffix) {
        // Require a directory separator immediately before the matched region.
        let prefix_len = file_path.len() - suffix.len();
        let boundary = file_path.as_bytes().get(prefix_len.saturating_sub(1)).copied();
        matches!(boundary, Some(b'/' | b'\\'))
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Bash language resolver
// ---------------------------------------------------------------------------

/// Bash language resolver.
pub struct BashResolver;

impl LanguageResolver for BashResolver {
    fn language_ids(&self) -> &[&str] {
        &["shell"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            // module_path keeps the stem (r.target_name) so resolve_common and
            // infer_external_common behave identically to baseline.  The raw path
            // (with $VAR/ or ./ prefixes) goes into `alias` for exclusive use by
            // resolve_via_shell_source's suffix-matching logic.
            let raw_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: Some(raw_path),
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "shell".to_string(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Bash builtins are never in the index.
        if predicates::is_bash_builtin(target) {
            return None;
        }

        // Deterministic resolution first: same-file lookup, scope chain, etc.
        if let Some(res) = engine::resolve_common("bash", file_ctx, ref_ctx, lookup, predicates::kind_compatible) {
            return Some(res);
        }

        // --- Shell source resolution (0.90) ---
        // Fallback for Calls refs that resolve_common could not find locally.
        // Shell's `source ./foo.sh` / `. $VAR/bar.sh` is a wildcard import:
        // ALL symbols from the target file enter scope.  Standard import-name
        // matching never fires here because the imported_name is the file stem,
        // not an individual function name.
        //
        // Strategy: for each sourced path in the file's import list, compute
        // the matchable suffix (stripping $VAR/ or ./ prefixes), then check
        // whether any candidate for `target` lives in a file whose path ends
        // with that suffix at a path-component boundary.
        if edge_kind == EdgeKind::Calls {
            if let Some(res) = self.resolve_via_shell_source(target, file_ctx, lookup) {
                return Some(res);
            }
        }

        // Bash bare-name fallback. Shell scripts call functions globally —
        // there's no per-file namespace once a script is sourced into the
        // shell. Standard module/import resolution can't bind these. The
        // counterpart is the SCSS resolver's `scss_bare_name` step (PR
        // 31): index-wide name lookup gated to bash-defined symbols.
        //
        // The dominant consumer is bash-completion library functions
        // (`_filedir`, `_init_completion`, `__git_*`, …) which the
        // `bash-completion-synthetics` ecosystem injects as ambient
        // globals. Pre-resolved-via-source step skips refs that we
        // already saw a `module` qualifier for.
        if edge_kind == EdgeKind::Calls && ref_ctx.extracted_ref.module.is_none() {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_bash = path.ends_with(".sh")
                    || path.ends_with(".bash")
                    || path.starts_with("ext:bash-completion-synthetics:");
                if !is_bash {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "bash_bare_name",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_bash_builtin)
    }
}

// ---------------------------------------------------------------------------
// Shell source resolution helper
// ---------------------------------------------------------------------------

impl BashResolver {
    /// Check whether `target_name` is defined in any file that this file
    /// sources via `source`/`.` directives.
    ///
    /// Returns `Some(Resolution)` at confidence 0.90 on the first match.
    fn resolve_via_shell_source(
        &self,
        target_name: &str,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        // Get all candidates indexed under this simple name.
        let candidates = lookup.by_name(target_name);
        if candidates.is_empty() {
            return None;
        }

        for import in &file_ctx.imports {
            // The raw path (with $VAR/ or ./ prefixes) is stored in the alias
            // field by build_file_context so infer_external_common's bare-name
            // walk sees the stem in module_path (unchanged from the baseline).
            let raw_path = match &import.alias {
                Some(p) => p.as_str(),
                None => continue,
            };
            // Only consider shell-file imports.
            if !raw_path.ends_with(".sh") && !raw_path.ends_with(".bash") {
                continue;
            }
            let suffix = shell_path_suffix(raw_path);
            if suffix.is_empty() {
                continue;
            }

            // Check if any candidate symbol lives in a file whose path
            // ends with this suffix at a directory-separator boundary.
            for sym in candidates {
                if ends_with_path_suffix(&sym.file_path, suffix) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.90,
                        strategy: "bash_shell_source",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        None
    }
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
