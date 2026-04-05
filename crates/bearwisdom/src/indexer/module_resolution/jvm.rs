// indexer/module_resolution/jvm.rs — JVM ecosystem resolver
// Handles Java, Kotlin, Scala, Groovy.
//
// Resolution rules:
//   1. `com.foo.Bar`  → `com/foo/Bar.java` (or .kt .scala .groovy),
//      suffix-matched against file_paths.
//   2. Wildcard `com.foo.*` → any file in the `com/foo/` directory.
//
// JVM files live under src/main/java, src/main/kotlin, etc., so we use
// suffix matching rather than root-anchored paths.

use super::ModuleResolver;

pub struct JvmModuleResolver;

const LANGUAGES: &[&str] = &["java", "kotlin", "scala", "groovy"];
const EXTENSIONS: &[&str] = &[".java", ".kt", ".scala", ".groovy"];

impl ModuleResolver for JvmModuleResolver {
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

        // Wildcard import: `com.foo.*`
        if specifier.ends_with(".*") {
            let pkg = &specifier[..specifier.len() - 2];
            let dir_suffix = pkg.replace('.', "/");
            return find_in_directory(&dir_suffix, file_paths);
        }

        // Exact type import: `com.foo.Bar`
        let path_stem = specifier.replace('.', "/");
        for ext in EXTENSIONS {
            let candidate = format!("{}{}", path_stem, ext);
            for &p in file_paths {
                let norm = p.replace('\\', "/");
                if norm.ends_with(&candidate) {
                    return Some(p.to_string());
                }
            }
        }

        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn find_in_directory(dir_suffix: &str, file_paths: &[&str]) -> Option<String> {
    let needle = format!("{}/", dir_suffix);
    for &p in file_paths {
        let norm = p.replace('\\', "/");
        if !EXTENSIONS.iter().any(|e| norm.ends_with(e)) {
            continue;
        }
        if norm.contains(&needle) {
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
        JvmModuleResolver.resolve_to_file(spec, from, files)
    }

    #[test]
    fn java_exact_import() {
        let files = &["src/main/java/com/example/service/UserService.java"];
        assert_eq!(
            resolve(
                "com.example.service.UserService",
                "src/main/java/com/example/App.java",
                files
            ),
            Some("src/main/java/com/example/service/UserService.java".into())
        );
    }

    #[test]
    fn kotlin_exact_import() {
        let files = &["src/main/kotlin/com/example/service/UserService.kt"];
        assert_eq!(
            resolve(
                "com.example.service.UserService",
                "src/main/kotlin/com/example/App.kt",
                files
            ),
            Some("src/main/kotlin/com/example/service/UserService.kt".into())
        );
    }

    #[test]
    fn wildcard_import() {
        let files = &[
            "src/main/java/com/example/service/UserService.java",
            "src/main/java/com/example/service/OrderService.java",
        ];
        let result = resolve(
            "com.example.service.*",
            "src/main/java/com/example/App.java",
            files,
        );
        assert!(result.is_some());
    }

    #[test]
    fn external_import_not_in_files() {
        let files: &[&str] = &[];
        assert!(resolve("org.springframework.boot.SpringApplication", "App.java", files).is_none());
    }
}
