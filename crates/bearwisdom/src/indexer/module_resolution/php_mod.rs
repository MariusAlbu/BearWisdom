// indexer/module_resolution/php_mod.rs — PHP PSR-4 resolver
//
// Resolution rules:
//   PSR-4: namespace `Foo\Bar\Baz` maps to a file path.
//   The composer.json autoload.psr-4 map defines namespace prefixes → base dirs.
//   Without parsing composer.json here, we use a heuristic:
//     - Convert `\` separators to `/`.
//     - Try the full path as a suffix: `Foo/Bar/Baz.php`.
//     - Also try with common src/ prefix stripped (projects typically mount
//       namespaces under `src/`, `app/`, `lib/`, or the root).
//
// If the namespace is not found in the indexed files, return None (external
// library from vendor/ or not indexed).

use super::ModuleResolver;

pub struct PhpModuleResolver;

const LANGUAGES: &[&str] = &["php"];

/// Common source root prefixes to try when doing suffix matching.
const SRC_PREFIXES: &[&str] = &["src/", "app/", "lib/", ""];

impl ModuleResolver for PhpModuleResolver {
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

        // Convert namespace separators and build path stem.
        // Strip a leading `\` if present (fully qualified name).
        let trimmed = specifier.trim_start_matches('\\');
        let path_stem = trimmed.replace('\\', "/");

        // Try `<stem>.php` as a suffix match.
        let candidate = format!("{}.php", path_stem);
        for &p in file_paths {
            let norm = p.replace('\\', "/");
            // Skip vendor directory — those are external.
            if is_vendor(&norm) {
                continue;
            }
            if norm.ends_with(&candidate) {
                return Some(p.to_string());
            }
        }

        // Try stripping a leading namespace component (PSR-4 prefix stripping heuristic).
        // e.g. `App\Services\UserService` → try `Services/UserService.php`.
        if let Some(slash_pos) = path_stem.find('/') {
            let without_prefix = &path_stem[slash_pos + 1..];
            let candidate2 = format!("{}.php", without_prefix);
            for prefix in SRC_PREFIXES {
                let full_candidate = format!("{}{}", prefix, candidate2);
                for &p in file_paths {
                    let norm = p.replace('\\', "/");
                    if is_vendor(&norm) {
                        continue;
                    }
                    if norm.ends_with(&full_candidate) || norm == full_candidate {
                        return Some(p.to_string());
                    }
                }
            }
        }

        None
    }
}

/// Returns true if the path is under a vendor directory.
fn is_vendor(norm: &str) -> bool {
    norm.starts_with("vendor/") || norm.contains("/vendor/")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(spec: &str, from: &str, files: &[&str]) -> Option<String> {
        PhpModuleResolver.resolve_to_file(spec, from, files)
    }

    #[test]
    fn psr4_suffix_match() {
        let files = &["src/Services/UserService.php"];
        assert_eq!(
            resolve(
                "App\\Services\\UserService",
                "src/Controllers/UserController.php",
                files
            ),
            Some("src/Services/UserService.php".into())
        );
    }

    #[test]
    fn fully_qualified_with_leading_backslash() {
        let files = &["src/Models/User.php"];
        assert_eq!(
            resolve("\\App\\Models\\User", "src/Controllers/UserController.php", files),
            Some("src/Models/User.php".into())
        );
    }

    #[test]
    fn vendor_is_excluded() {
        let files = &["vendor/laravel/framework/src/Illuminate/Support/Str.php"];
        assert!(resolve(
            "Illuminate\\Support\\Str",
            "src/Controllers/UserController.php",
            files
        )
        .is_none());
    }

    #[test]
    fn external_returns_none() {
        let files: &[&str] = &[];
        assert!(resolve(
            "Illuminate\\Support\\Facades\\Route",
            "routes/web.php",
            files
        )
        .is_none());
    }
}
