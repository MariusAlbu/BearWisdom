//! Integration and unit tests for bearwisdom-profile.
//!
//! All tests are in one file to keep tempfile fixture helpers centralized.

#[cfg(test)]
mod detection {
    use crate::detect::detect_language;
    use std::path::Path;

    #[test]
    fn rust_by_extension() {
        let lang = detect_language(Path::new("main.rs")).expect("should detect");
        assert_eq!(lang.id, "rust");
    }

    #[test]
    fn typescript_ts() {
        let lang = detect_language(Path::new("app.ts")).expect("should detect");
        assert_eq!(lang.id, "typescript");
    }

    #[test]
    fn typescript_tsx() {
        let lang = detect_language(Path::new("Component.tsx")).expect("should detect");
        assert_eq!(lang.id, "typescript");
    }

    #[test]
    fn javascript_mjs() {
        let lang = detect_language(Path::new("mod.mjs")).expect("should detect");
        assert_eq!(lang.id, "javascript");
    }

    #[test]
    fn csharp() {
        let lang = detect_language(Path::new("Program.cs")).expect("should detect");
        assert_eq!(lang.id, "csharp");
    }

    #[test]
    fn python() {
        let lang = detect_language(Path::new("main.py")).expect("should detect");
        assert_eq!(lang.id, "python");
    }

    #[test]
    fn go_extension() {
        let lang = detect_language(Path::new("server.go")).expect("should detect");
        assert_eq!(lang.id, "go");
    }

    #[test]
    fn java_extension() {
        let lang = detect_language(Path::new("App.java")).expect("should detect");
        assert_eq!(lang.id, "java");
    }

    #[test]
    fn kotlin_extension() {
        let lang = detect_language(Path::new("Main.kt")).expect("should detect");
        assert_eq!(lang.id, "kotlin");
    }

    #[test]
    fn swift_extension() {
        let lang = detect_language(Path::new("ContentView.swift")).expect("should detect");
        assert_eq!(lang.id, "swift");
    }

    #[test]
    fn ruby_extension() {
        let lang = detect_language(Path::new("config.rb")).expect("should detect");
        assert_eq!(lang.id, "ruby");
    }

    #[test]
    fn php_extension() {
        let lang = detect_language(Path::new("index.php")).expect("should detect");
        assert_eq!(lang.id, "php");
    }

    #[test]
    fn c_extension() {
        let lang = detect_language(Path::new("main.c")).expect("should detect");
        assert_eq!(lang.id, "c");
    }

    #[test]
    fn cpp_extension() {
        let lang = detect_language(Path::new("engine.cpp")).expect("should detect");
        assert_eq!(lang.id, "cpp");
    }

    #[test]
    fn cpp_header() {
        let lang = detect_language(Path::new("types.hpp")).expect("should detect");
        assert_eq!(lang.id, "cpp");
    }

    #[test]
    fn html_extension() {
        let lang = detect_language(Path::new("index.html")).expect("should detect");
        assert_eq!(lang.id, "html");
    }

    #[test]
    fn css_extension() {
        let lang = detect_language(Path::new("styles.css")).expect("should detect");
        assert_eq!(lang.id, "css");
    }

    #[test]
    fn scss_extension() {
        let lang = detect_language(Path::new("theme.scss")).expect("should detect");
        assert_eq!(lang.id, "scss");
    }

    #[test]
    fn json_extension() {
        let lang = detect_language(Path::new("config.json")).expect("should detect");
        assert_eq!(lang.id, "json");
    }

    #[test]
    fn yaml_extension() {
        let lang = detect_language(Path::new("ci.yml")).expect("should detect");
        assert_eq!(lang.id, "yaml");
    }

    #[test]
    fn xml_extension() {
        let lang = detect_language(Path::new("pom.xml")).expect("should detect");
        assert_eq!(lang.id, "xml");
    }

    #[test]
    fn markdown_extension() {
        let lang = detect_language(Path::new("README.md")).expect("should detect");
        assert_eq!(lang.id, "markdown");
    }

    #[test]
    fn sql_extension() {
        let lang = detect_language(Path::new("migration.sql")).expect("should detect");
        assert_eq!(lang.id, "sql");
    }

    #[test]
    fn shell_extension() {
        let lang = detect_language(Path::new("deploy.sh")).expect("should detect");
        assert_eq!(lang.id, "shell");
    }

    #[test]
    fn dockerfile_by_filename() {
        let lang = detect_language(Path::new("Dockerfile")).expect("should detect");
        assert_eq!(lang.id, "dockerfile");
    }

    #[test]
    fn dockerfile_by_extension() {
        let lang = detect_language(Path::new("app.dockerfile")).expect("should detect");
        assert_eq!(lang.id, "dockerfile");
    }

    #[test]
    fn toml_extension() {
        let lang = detect_language(Path::new("Cargo.toml")).expect("should detect");
        assert_eq!(lang.id, "toml");
    }

    #[test]
    fn unknown_returns_none() {
        assert!(detect_language(Path::new("image.png")).is_none());
        assert!(detect_language(Path::new("font.woff2")).is_none());
        assert!(detect_language(Path::new("binary.exe")).is_none());
        assert!(detect_language(Path::new("noextension")).is_none());
    }
}

#[cfg(test)]
mod exclusions_tests {
    use crate::exclusions::{canonical_exclude_dirs, should_exclude};

    #[test]
    fn node_modules_excluded() {
        assert!(should_exclude("node_modules"));
    }

    #[test]
    fn target_excluded() {
        assert!(should_exclude("target"));
    }

    #[test]
    fn venv_excluded() {
        assert!(should_exclude(".venv"));
    }

    #[test]
    fn git_excluded() {
        assert!(should_exclude(".git"));
    }

    #[test]
    fn bin_obj_excluded() {
        assert!(should_exclude("bin"));
        assert!(should_exclude("obj"));
    }

    #[test]
    fn src_not_excluded() {
        assert!(!should_exclude("src"));
    }

    #[test]
    fn lib_not_excluded() {
        assert!(!should_exclude("lib"));
    }

    #[test]
    fn canonical_contains_all_language_dirs() {
        let dirs = canonical_exclude_dirs();
        assert!(dirs.contains(&"target"));
        assert!(dirs.contains(&"node_modules"));
        assert!(dirs.contains(&".venv"));
        assert!(dirs.contains(&"vendor"));
        assert!(dirs.contains(&"bin"));
        assert!(dirs.contains(&"obj"));
        assert!(dirs.contains(&"build"));
        assert!(dirs.contains(&"__pycache__"));
    }

    #[test]
    fn canonical_no_duplicates() {
        let dirs = canonical_exclude_dirs();
        let mut sorted = dirs.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(dirs.len(), sorted.len(), "canonical_exclude_dirs contains duplicates");
    }
}

#[cfg(test)]
mod registry_tests {
    use crate::registry::{find_language, find_language_by_extension, LANGUAGES};

    #[test]
    fn languages_count() {
        assert_eq!(LANGUAGES.len(), 86, "expected 86 language descriptors");
    }

    #[test]
    fn find_by_id() {
        assert!(find_language("rust").is_some());
        assert!(find_language("typescript").is_some());
        assert!(find_language("csharp").is_some());
    }

    #[test]
    fn find_by_alias() {
        // "ts" is an alias for typescript
        assert!(find_language("ts").is_some());
        // "golang" is an alias for go
        assert!(find_language("golang").is_some());
    }

    #[test]
    fn find_by_extension() {
        let lang = find_language_by_extension(".rs").expect("should find rust");
        assert_eq!(lang.id, "rust");
    }

    #[test]
    fn find_by_extension_without_dot() {
        let lang = find_language_by_extension("rs").expect("should normalise dot");
        assert_eq!(lang.id, "rust");
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert!(find_language_by_extension(".xyz").is_none());
    }

    #[test]
    fn all_languages_have_unique_ids() {
        let ids: Vec<&str> = LANGUAGES.iter().map(|l| l.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "duplicate language ids found");
    }

    #[test]
    fn all_extensions_start_with_dot() {
        for lang in LANGUAGES {
            for ext in lang.file_extensions {
                assert!(
                    ext.starts_with('.'),
                    "extension `{ext}` for language `{}` must start with a dot",
                    lang.id
                );
            }
        }
    }
}

#[cfg(test)]
mod scanner_tests {
    use crate::{scan, ScanOptions};
    use std::fs;
    use tempfile::TempDir;

    fn options_no_sdk() -> ScanOptions {
        ScanOptions { check_sdks: false, max_depth: 3 }
    }

    fn write(dir: &TempDir, rel: &str, content: &str) {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // ------------------------------------------------------------------
    // Rust project
    // ------------------------------------------------------------------
    #[test]
    fn rust_project_detected() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "Cargo.toml", "[package]\nname = \"hello\"\n");
        write(&tmp, "src/main.rs", "fn main() {}");

        let profile = scan(tmp.path(), options_no_sdk());
        let rust = profile.languages.iter().find(|l| l.language_id == "rust");
        assert!(rust.is_some(), "rust should be detected");
        let rust = rust.unwrap();
        assert!(rust.file_count >= 1);
        assert!(
            rust.entry_points.iter().any(|ep| ep.contains("Cargo.toml")),
            "Cargo.toml should be an entry point"
        );
    }

    // ------------------------------------------------------------------
    // Node/TypeScript project
    // ------------------------------------------------------------------
    #[test]
    fn typescript_project_detected() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "package.json", r#"{"name":"app","dependencies":{}}"#);
        write(&tmp, "tsconfig.json", r#"{"compilerOptions":{}}"#);
        write(&tmp, "src/index.ts", "export const x = 1;");
        write(&tmp, "src/app.tsx", "export const App = () => null;");

        let profile = scan(tmp.path(), options_no_sdk());
        let ts = profile.languages.iter().find(|l| l.language_id == "typescript");
        assert!(ts.is_some(), "typescript should be detected");
        let ts = ts.unwrap();
        assert!(ts.file_count >= 2);
        assert!(
            ts.entry_points.iter().any(|ep| ep.contains("package.json")),
            "package.json should be an entry point"
        );
    }

    #[test]
    fn npm_package_manager_detected() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "package.json", r#"{"name":"app"}"#);
        write(&tmp, "package-lock.json", "{}");
        write(&tmp, "src/index.ts", "export {};");

        let profile = scan(tmp.path(), options_no_sdk());
        let npm = profile
            .package_managers
            .iter()
            .find(|pm| pm.name == "npm");
        assert!(npm.is_some(), "npm should be detected");
        assert!(npm.unwrap().has_lock_file, "lock file should be present");
    }

    #[test]
    fn node_modules_missing_flagged_in_restore() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "package.json", r#"{"name":"app"}"#);
        write(&tmp, "package-lock.json", "{}");
        write(&tmp, "src/index.ts", "export {};");
        // Deliberately NOT creating node_modules/

        let profile = scan(tmp.path(), options_no_sdk());
        let has_restore = profile
            .restore_steps
            .iter()
            .any(|s| s.contains("npm") || s.contains("deps"));
        assert!(has_restore, "missing node_modules should produce a restore step");
    }

    // ------------------------------------------------------------------
    // C# project
    // ------------------------------------------------------------------
    #[test]
    fn csharp_project_detected() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "MyApp.sln", "Microsoft Visual Studio Solution File");
        write(&tmp, "MyApp/MyApp.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\"></Project>");
        write(&tmp, "MyApp/Program.cs", "Console.WriteLine(\"hello\");");

        let profile = scan(tmp.path(), options_no_sdk());
        let cs = profile.languages.iter().find(|l| l.language_id == "csharp");
        assert!(cs.is_some(), "csharp should be detected");
    }

    // ------------------------------------------------------------------
    // Python project
    // ------------------------------------------------------------------
    #[test]
    fn python_project_detected() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "requirements.txt", "requests==2.28.0\npytest==7.0.0\n");
        write(&tmp, "main.py", "print('hello')");
        write(&tmp, "tests/test_main.py", "def test_hello(): pass");

        let profile = scan(tmp.path(), options_no_sdk());
        let py = profile.languages.iter().find(|l| l.language_id == "python");
        assert!(py.is_some(), "python should be detected");
        let py = py.unwrap();
        assert!(py.file_count >= 2);
        assert!(
            py.entry_points.iter().any(|ep| ep.contains("requirements.txt")),
            "requirements.txt should be an entry point"
        );
    }

    #[test]
    fn python_venv_missing_flagged() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "requirements.txt", "requests\n");
        write(&tmp, "main.py", "print('hi')");
        // No .venv directory

        let profile = scan(tmp.path(), options_no_sdk());
        let has_restore = profile
            .restore_steps
            .iter()
            .any(|s| s.to_lowercase().contains("venv") || s.to_lowercase().contains("virtual"));
        assert!(has_restore, "missing .venv should trigger a restore step");
    }

    // ------------------------------------------------------------------
    // Mixed project
    // ------------------------------------------------------------------
    #[test]
    fn mixed_project_multiple_languages() {
        let tmp = TempDir::new().unwrap();
        // Rust backend
        write(&tmp, "src-tauri/Cargo.toml", "[package]\nname=\"backend\"\n");
        write(&tmp, "src-tauri/src/main.rs", "fn main() {}");
        // TypeScript frontend
        write(&tmp, "package.json", r#"{"name":"frontend"}"#);
        write(&tmp, "src/App.tsx", "export const App = () => null;");
        write(&tmp, "src/index.ts", "export {};");

        let profile = scan(tmp.path(), options_no_sdk());
        let has_rust = profile.languages.iter().any(|l| l.language_id == "rust");
        let has_ts = profile.languages.iter().any(|l| l.language_id == "typescript");
        assert!(has_rust, "rust should be detected in mixed project");
        assert!(has_ts, "typescript should be detected in mixed project");
    }

    // ------------------------------------------------------------------
    // Exclusion: files inside excluded dirs don't count
    // ------------------------------------------------------------------
    #[test]
    fn excluded_dirs_are_skipped() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "package.json", r#"{"name":"app"}"#);
        write(&tmp, "src/index.ts", "export {};");
        // A large "file" inside node_modules — should NOT be counted
        write(&tmp, "node_modules/big-library/index.js", "module.exports = {};");
        // Also put a fake .rs in target/
        write(&tmp, "Cargo.toml", "[package]\nname=\"test\"\n");
        write(&tmp, "target/debug/build/some.rs", "fn main() {}");

        let profile = scan(tmp.path(), options_no_sdk());

        // TypeScript count should be 1 (only src/index.ts), not counting node_modules.
        let ts = profile.languages.iter().find(|l| l.language_id == "typescript");
        // We don't assert exact count because the walker may also pick up package.json → json,
        // but the JS file inside node_modules must NOT be counted.
        // Indirect check: language stats should not contain "javascript" driven purely
        // by the node_modules file.
        if let Some(js) = profile.languages.iter().find(|l| l.language_id == "javascript") {
            assert_eq!(
                js.file_count, 0,
                "node_modules/big-library/index.js must not be counted"
            );
        }

        // The Rust file inside target/ must not be counted.
        let rust = profile.languages.iter().find(|l| l.language_id == "rust");
        assert!(
            rust.as_ref().map(|r| r.file_count).unwrap_or(0) == 0,
            "target/ Rust files must not be counted"
        );

        let _ = ts; // suppress unused warning
    }

    // ------------------------------------------------------------------
    // Environment detection
    // ------------------------------------------------------------------
    #[test]
    fn env_example_without_env_flagged() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, ".env.example", "DATABASE_URL=postgres://localhost/dev");
        write(&tmp, "package.json", r#"{"name":"app"}"#);
        write(&tmp, "src/index.ts", "export {};");

        let profile = scan(tmp.path(), options_no_sdk());
        assert!(
            profile.environment.missing_env_file,
            ".env.example without .env should set missing_env_file"
        );
        assert!(
            profile.restore_steps.iter().any(|s| s.contains(".env")),
            "missing .env should produce a restore step"
        );
    }

    #[test]
    fn docker_compose_detected() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "docker-compose.yml", "version: '3'\nservices:\n  app:\n    image: nginx\n");
        write(&tmp, "Dockerfile", "FROM nginx");
        write(&tmp, "src/main.rs", "fn main() {}");

        let profile = scan(tmp.path(), options_no_sdk());
        assert!(profile.environment.has_docker_compose);
    }

    // ------------------------------------------------------------------
    // Monorepo detection
    // ------------------------------------------------------------------
    #[test]
    fn cargo_workspace_detected() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp,
            "Cargo.toml",
            "[workspace]\nmembers = [\n  \"crates/core\",\n  \"crates/cli\",\n]\n",
        );
        write(&tmp, "crates/core/Cargo.toml", "[package]\nname=\"core\"\n");
        write(&tmp, "crates/core/src/lib.rs", "pub fn hello() {}");

        let profile = scan(tmp.path(), options_no_sdk());
        assert!(profile.monorepo.is_some(), "cargo workspace should be detected");
        assert_eq!(profile.monorepo.as_ref().unwrap().kind, "cargo-workspace");
    }

    #[test]
    fn npm_workspace_detected() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp,
            "package.json",
            r#"{"name":"monorepo","workspaces":["packages/*"]}"#,
        );
        write(&tmp, "packages/ui/package.json", r#"{"name":"@app/ui"}"#);
        write(&tmp, "packages/ui/src/index.ts", "export {};");

        let profile = scan(tmp.path(), options_no_sdk());
        assert!(profile.monorepo.is_some(), "npm workspace should be detected");
        assert_eq!(profile.monorepo.as_ref().unwrap().kind, "npm-workspaces");
    }

    // ------------------------------------------------------------------
    // Test framework detection
    // ------------------------------------------------------------------
    #[test]
    fn vitest_detected_from_config() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp,
            "vite.config.ts",
            r#"import { defineConfig } from "vite"; export default defineConfig({ test: { vitest: true } });"#,
        );
        write(&tmp, "package.json", r#"{"name":"app","devDependencies":{"vitest":"^1.0"}}"#);
        write(&tmp, "src/app.ts", "export const x = 1;");

        let profile = scan(tmp.path(), options_no_sdk());
        let vitest = profile
            .test_frameworks
            .iter()
            .find(|tf| tf.name == "vitest");
        assert!(vitest.is_some(), "vitest should be detected from vite.config.ts");
    }

    #[test]
    fn pytest_detected_from_config() {
        let tmp = TempDir::new().unwrap();
        write(&tmp, "pyproject.toml", "[tool.pytest.ini_options]\ntestpaths = [\"tests\"]\n");
        write(&tmp, "main.py", "print('hello')");
        write(&tmp, "tests/test_foo.py", "def test_foo(): pass");

        let profile = scan(tmp.path(), options_no_sdk());
        let pytest = profile
            .test_frameworks
            .iter()
            .find(|tf| tf.name == "pytest");
        assert!(pytest.is_some(), "pytest should be detected from pyproject.toml");
    }
}

#[cfg(test)]
mod shell_commands_tests {
    use crate::types::ShellCommands;

    #[test]
    fn same_sets_all_shells() {
        let cmd = ShellCommands::same("cargo test");
        assert_eq!(cmd.bash, "cargo test");
        assert_eq!(cmd.powershell, "cargo test");
        assert_eq!(cmd.cmd, "cargo test");
    }

    #[test]
    fn different_shells() {
        let cmd = ShellCommands {
            bash: "source .venv/bin/activate",
            powershell: ".venv\\Scripts\\Activate.ps1",
            cmd: ".venv\\Scripts\\activate.bat",
        };
        assert_ne!(cmd.bash, cmd.powershell);
    }
}
