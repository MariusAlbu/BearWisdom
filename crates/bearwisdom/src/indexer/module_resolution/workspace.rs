// =============================================================================
// module_resolution/workspace — declared workspace packages resolve for every
// language
//
// A specifier whose head names a workspace package (`next/link` in a repo
// whose `packages/next` declares `name: next`; `tantivy::schema` in a Cargo
// workspace member) maps to a file under that package's root directory. The
// mapping is manifest-driven — declared name → package root — and identical
// across languages, so this resolver is universal (empty `language_ids`) and
// runs before the per-language resolver.
// =============================================================================

use super::ModuleResolver;

pub struct WorkspacePackageResolver {
    /// Declared package name → package root directory (project-relative,
    /// forward slashes, no trailing slash).
    packages: Vec<(String, String)>,
}

impl WorkspacePackageResolver {
    pub fn new(packages: Vec<(String, String)>) -> Self {
        Self { packages }
    }
}

impl ModuleResolver for WorkspacePackageResolver {
    fn language_ids(&self) -> &[&str] {
        &[]
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        _importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        if specifier.starts_with('.') || specifier.starts_with('/') {
            return None;
        }
        // `::` (qualified-path separator) canonicalizes to `/` so a deep
        // `member::module` specifier peels the same way `pkg/sub` does.
        let normalized = specifier.replace("::", "/");
        // Longest declared name owning the specifier head wins.
        let (name, root) = self
            .packages
            .iter()
            .filter(|(name, _)| {
                normalized == *name
                    || normalized
                        .strip_prefix(name.as_str())
                        .is_some_and(|r| r.starts_with('/'))
            })
            .max_by_key(|(name, _)| name.len())?;
        let sub = normalized[name.len()..].trim_start_matches('/');
        let root_prefix = format!("{root}/");
        let mut best: Option<&str> = None;
        for path in file_paths {
            let Some(rest) = path.strip_prefix(root_prefix.as_str()) else {
                continue;
            };
            if !sub_matches(rest, sub) {
                continue;
            }
            // Shortest path is the most direct mapping; ties break
            // lexicographically for determinism.
            if best.is_none_or(|b| (path.len(), *path) < (b.len(), b)) {
                best = Some(path);
            }
        }
        best.map(str::to_string)
    }
}

/// Whether a package-relative file path serves the specifier's sub-path: the
/// path equals the sub-path, or does after shedding extensions (`link.d.ts`
/// serves `link`), or is the sub-path's `index` file. An empty sub-path (the
/// specifier IS the package) is served only by a root-level `index` file —
/// resolving a bare package to an arbitrary file would claim more than the
/// manifest says.
fn sub_matches(rest: &str, sub: &str) -> bool {
    if !sub.is_empty() && rest == sub {
        return true;
    }
    let index_form;
    let want_index: &str = if sub.is_empty() {
        "index"
    } else {
        index_form = format!("{sub}/index");
        &index_form
    };
    let mut stem = rest;
    loop {
        let next = strip_extension(stem);
        if next == stem {
            return false;
        }
        stem = next;
        if (!sub.is_empty() && stem == sub) || stem == want_index {
            return true;
        }
    }
}

/// The path without the final segment's last extension (`a/b.d.ts` → `a/b.d`).
/// Returns the input unchanged when the final segment has no `.`.
fn strip_extension(path: &str) -> &str {
    match (path.rfind('/'), path.rfind('.')) {
        (_, None) => path,
        (Some(slash), Some(dot)) if dot < slash => path,
        (_, Some(dot)) => &path[..dot],
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
