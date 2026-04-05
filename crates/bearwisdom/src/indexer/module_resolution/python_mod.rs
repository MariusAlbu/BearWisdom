// indexer/module_resolution/python_mod.rs — Python module resolver
//
// Resolution rules:
//   1. Absolute: `foo.bar.baz` → convert `.` → `/`, try `foo/bar/baz.py`
//      and `foo/bar/baz/__init__.py` (suffix match against file_paths).
//   2. Relative: `.foo` (one leading dot = current package),
//      `..foo` (two dots = parent package), etc.
//      Strip dots, resolve relative to importing file's package directory.

use super::ModuleResolver;

pub struct PythonModuleResolver;

const LANGUAGES: &[&str] = &["python"];

impl ModuleResolver for PythonModuleResolver {
    fn language_ids(&self) -> &[&str] {
        LANGUAGES
    }

    fn resolve_to_file(
        &self,
        specifier: &str,
        importing_file: &str,
        file_paths: &[&str],
    ) -> Option<String> {
        if specifier.is_empty() {
            return None;
        }

        // Count leading dots for relative imports.
        let leading_dots = specifier.chars().take_while(|&c| c == '.').count();

        if leading_dots > 0 {
            // Relative import.
            let module_part = &specifier[leading_dots..];
            let base_dir = package_dir(importing_file, leading_dots);
            let rel = if module_part.is_empty() {
                String::new()
            } else {
                module_part.replace('.', "/")
            };
            let base = if rel.is_empty() {
                base_dir
            } else if base_dir.is_empty() {
                rel
            } else {
                format!("{}/{}", base_dir, rel)
            };
            find_python_module(&normalise_path(&base), file_paths)
        } else {
            // Absolute import: dots as separators.
            let rel = specifier.replace('.', "/");
            find_python_module(&rel, file_paths)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the package directory for an importing file, going up `dots` levels.
/// One dot = stay in the file's directory; two dots = go one level up; etc.
fn package_dir(file_path: &str, dots: usize) -> String {
    let norm = file_path.replace('\\', "/");
    let dir = if let Some(pos) = norm.rfind('/') {
        norm[..pos].to_string()
    } else {
        String::new()
    };

    // Each additional dot beyond 1 means go one directory up.
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for _ in 1..dots {
        parts.pop();
    }
    parts.join("/")
}

fn normalise_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let mut out: Vec<&str> = Vec::with_capacity(parts.len());
    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

/// Try `<rel>.py` and `<rel>/__init__.py` as suffix matches.
fn find_python_module(rel: &str, file_paths: &[&str]) -> Option<String> {
    if rel.is_empty() {
        return None;
    }
    let candidate_py = format!("{}.py", rel);
    let candidate_init = format!("{}/__init__.py", rel);

    for &p in file_paths {
        let norm = p.replace('\\', "/");
        if norm.ends_with(&candidate_py) || norm == candidate_py {
            return Some(p.to_string());
        }
        if norm.ends_with(&candidate_init) || norm == candidate_init {
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

    fn resolve(spec: &str, from: &str, files: &[&str]) -> Option<String> {
        PythonModuleResolver.resolve_to_file(spec, from, files)
    }

    #[test]
    fn absolute_module() {
        let files = &["myapp/services/user.py"];
        assert_eq!(
            resolve("myapp.services.user", "myapp/main.py", files),
            Some("myapp/services/user.py".into())
        );
    }

    #[test]
    fn absolute_package_init() {
        let files = &["myapp/services/__init__.py"];
        assert_eq!(
            resolve("myapp.services", "myapp/main.py", files),
            Some("myapp/services/__init__.py".into())
        );
    }

    #[test]
    fn relative_single_dot() {
        let files = &["myapp/services/helpers.py"];
        assert_eq!(
            resolve(".helpers", "myapp/services/user.py", files),
            Some("myapp/services/helpers.py".into())
        );
    }

    #[test]
    fn relative_double_dot() {
        let files = &["myapp/utils.py"];
        assert_eq!(
            resolve("..utils", "myapp/services/user.py", files),
            Some("myapp/utils.py".into())
        );
    }

    #[test]
    fn stdlib_is_not_resolved() {
        // os is not in our file set — should return None (not crash).
        let files: &[&str] = &[];
        assert!(resolve("os", "myapp/main.py", files).is_none());
    }
}
