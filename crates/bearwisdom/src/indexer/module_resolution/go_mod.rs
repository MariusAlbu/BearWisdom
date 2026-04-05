// indexer/module_resolution/go_mod.rs — Go module resolver
//
// Resolution rules:
//   Go imports are full module paths (e.g. "github.com/user/repo/pkg/foo").
//   The last segment is the package directory name.
//
//   1. If `module_path` is provided and the specifier starts with that prefix,
//      it is an internal import.  Strip the module prefix, treat the rest as a
//      relative directory path, and find any `.go` file in that directory.
//   2. Otherwise, check whether any indexed file lives in a directory whose
//      suffix matches the import path's last component(s).  This handles repos
//      where the module path isn't known.
//   3. External imports (no matching directory) → return None.

use super::ModuleResolver;

pub struct GoModuleResolver {
    /// The module path from go.mod (e.g. "github.com/gitea/gitea").
    /// Optional — callers that have `ProjectContext` should provide it.
    module_path: Option<String>,
}

impl GoModuleResolver {
    pub fn new(module_path: Option<String>) -> Self {
        Self { module_path }
    }
}

const LANGUAGES: &[&str] = &["go"];

impl ModuleResolver for GoModuleResolver {
    fn language_ids(&self) -> &[&str] {
        LANGUAGES
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        _importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        if specifier.is_empty() {
            return None;
        }

        // Normalise the import path (forward slashes).
        let spec = specifier.replace('\\', "/");

        // If we have the go.mod module path, check whether this is an internal import.
        if let Some(ref mod_path) = self.module_path {
            if let Some(tail) = spec.strip_prefix(mod_path.as_str()) {
                // `tail` is either empty (importing the root package) or starts with `/`.
                let pkg_dir = tail.trim_start_matches('/');
                return find_go_file_in_dir(pkg_dir, file_paths);
            }
            // Doesn't match module prefix → external.
            return None;
        }

        // No module path available: try to match by directory suffix.
        // The last segment of the import path is the package directory.
        let pkg_suffix = spec.trim_end_matches('/');
        find_go_file_in_dir(pkg_suffix, file_paths)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find any `.go` file that lives in `pkg_dir` (suffix match on directory part).
fn find_go_file_in_dir(pkg_dir: &str, file_paths: &[&str]) -> Option<String> {
    if pkg_dir.is_empty() {
        return None;
    }

    // Normalise expected directory suffix (no trailing slash).
    let dir_suffix = pkg_dir.trim_end_matches('/');

    for &p in file_paths {
        let norm = p.replace('\\', "/");
        if !norm.ends_with(".go") {
            continue;
        }
        // Get the directory portion of the file.
        let file_dir = if let Some(pos) = norm.rfind('/') {
            &norm[..pos]
        } else {
            // File at repo root.
            ""
        };

        if file_dir == dir_suffix || file_dir.ends_with(&format!("/{}", dir_suffix)) {
            return Some(p.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_resolver(mod_path: Option<&str>) -> GoModuleResolver {
        GoModuleResolver::new(mod_path.map(str::to_string))
    }

    fn resolve(
        resolver: &GoModuleResolver,
        spec: &str,
        from: &str,
        files: &[&str],
    ) -> Option<String> {
        resolver.resolve_to_file(spec, from, files)
    }

    #[test]
    fn internal_with_module_path() {
        let r = make_resolver(Some("github.com/my/app"));
        let files = &["pkg/user/user.go", "pkg/user/repo.go"];
        let result = resolve(&r, "github.com/my/app/pkg/user", "main.go", files);
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert!(resolved.ends_with(".go"));
    }

    #[test]
    fn external_with_module_path() {
        let r = make_resolver(Some("github.com/my/app"));
        let files = &["vendor/github.com/gin-gonic/gin/gin.go"];
        assert!(resolve(&r, "github.com/gin-gonic/gin", "main.go", files).is_none());
    }

    #[test]
    fn no_module_path_suffix_match() {
        let r = make_resolver(None);
        let files = &["internal/services/user/user.go"];
        let result = resolve(&r, "internal/services/user", "main.go", files);
        assert_eq!(result, Some("internal/services/user/user.go".into()));
    }

    #[test]
    fn empty_specifier() {
        let r = make_resolver(None);
        assert!(resolve(&r, "", "main.go", &[]).is_none());
    }
}
