// =============================================================================
// walker.rs  —  gitignore-aware file discovery
//
// Delegates entirely to `bearwisdom_profile::walk_files`, which owns the
// walk logic (UNC stripping, OverrideBuilder exclusions, gitignore, sorting).
// This module exists to provide the `WalkedFile` type used by the indexer and
// the `detect_language` helper used by the parser layer.
// =============================================================================

use bearwisdom_profile::detect_language as profile_detect_language;
use bearwisdom_profile::ScannedFile;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// A file that was found and is ready to be parsed.
#[derive(Debug, Clone)]
pub struct WalkedFile {
    /// Path as stored in the DB (relative to project root, forward slashes).
    pub relative_path: String,
    /// Absolute path — used for reading file contents.
    pub absolute_path: PathBuf,
    /// Language identifier (e.g. "csharp", "typescript").
    pub language: &'static str,
}

impl From<&ScannedFile> for WalkedFile {
    fn from(sf: &ScannedFile) -> Self {
        WalkedFile {
            relative_path: sf.relative_path.clone(),
            absolute_path: sf.absolute_path.clone(),
            language: sf.language_id,
        }
    }
}

/// Walk `project_root` and return all indexable source files.
///
/// Delegates to `bearwisdom_profile::walk_files` for all discovery logic.
/// Files are sorted by relative path for deterministic output across OSes.
pub fn walk(project_root: &Path) -> Result<Vec<WalkedFile>> {
    let scanned = bearwisdom_profile::walk_files(project_root);
    Ok(scanned.iter().map(WalkedFile::from).collect())
}

/// Map a file path to a language identifier.
///
/// Returns `None` for paths we don't support so the caller can skip the file.
/// Delegates to `bearwisdom-profile` for all detection logic, preserving
/// the `Option<&'static str>` return type expected by callers.
pub fn detect_language(path: &Path) -> Option<&'static str> {
    profile_detect_language(path).map(|desc| desc.id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn detect_csharp() {
        assert_eq!(detect_language(Path::new("Program.cs")), Some("csharp"));
    }

    #[test]
    fn detect_typescript() {
        assert_eq!(detect_language(Path::new("api.ts")), Some("typescript"));
        // .tsx maps to the "typescript" language in bearwisdom-profile
        assert_eq!(detect_language(Path::new("App.tsx")), Some("typescript"));
    }

    #[test]
    fn detect_python() {
        assert_eq!(detect_language(Path::new("main.py")), Some("python"));
        assert_eq!(detect_language(Path::new("script.pyw")), Some("python"));
    }

    #[test]
    fn detect_compiled_languages() {
        assert_eq!(detect_language(Path::new("Main.java")), Some("java"));
        assert_eq!(detect_language(Path::new("main.go")), Some("go"));
        assert_eq!(detect_language(Path::new("lib.rs")), Some("rust"));
        assert_eq!(detect_language(Path::new("app.rb")), Some("ruby"));
        assert_eq!(detect_language(Path::new("index.php")), Some("php"));
        assert_eq!(detect_language(Path::new("main.c")), Some("c"));
        assert_eq!(detect_language(Path::new("header.h")), Some("c"));
        assert_eq!(detect_language(Path::new("main.cpp")), Some("cpp"));
        assert_eq!(detect_language(Path::new("main.cc")), Some("cpp"));
        assert_eq!(detect_language(Path::new("header.hpp")), Some("cpp"));
        assert_eq!(detect_language(Path::new("Main.kt")), Some("kotlin"));
        assert_eq!(detect_language(Path::new("App.swift")), Some("swift"));
        // Languages not yet in bearwisdom-profile registry — return None via profile crate.
        assert_eq!(detect_language(Path::new("Main.scala")), None);
        assert_eq!(detect_language(Path::new("main.dart")), None);
        assert_eq!(detect_language(Path::new("lib.ex")), None);
        assert_eq!(detect_language(Path::new("script.exs")), None);
        assert_eq!(detect_language(Path::new("init.lua")), None);
        assert_eq!(detect_language(Path::new("analysis.r")), None);
        assert_eq!(detect_language(Path::new("Analysis.R")), None);
        assert_eq!(detect_language(Path::new("Main.hs")), None);
    }

    #[test]
    fn detect_markup_config_data() {
        assert_eq!(detect_language(Path::new("index.html")), Some("html"));
        assert_eq!(detect_language(Path::new("page.htm")), Some("html"));
        assert_eq!(detect_language(Path::new("style.css")), Some("css"));
        assert_eq!(detect_language(Path::new("vars.scss")), Some("scss"));
        assert_eq!(detect_language(Path::new("data.json")), Some("json"));
        assert_eq!(detect_language(Path::new("config.yml")), Some("yaml"));
        assert_eq!(detect_language(Path::new("config.yaml")), Some("yaml"));
        assert_eq!(detect_language(Path::new("data.xml")), Some("xml"));
        assert_eq!(detect_language(Path::new("transform.xsl")), Some("xml"));
        assert_eq!(detect_language(Path::new("README.md")), Some("markdown"));
        // Shell files map to "shell" in bearwisdom-profile (not "bash").
        assert_eq!(detect_language(Path::new("deploy.sh")), Some("shell"));
        assert_eq!(detect_language(Path::new("run.bash")), Some("shell"));
        assert_eq!(detect_language(Path::new("profile.zsh")), Some("shell"));
    }

    #[test]
    fn detect_dockerfile() {
        assert_eq!(detect_language(Path::new("Dockerfile")), Some("dockerfile"));
        // bearwisdom-profile matches exact filenames only — Dockerfile.* variants
        // are not in the filenames list, so they return None.
        assert_eq!(detect_language(Path::new("Dockerfile.prod")), None);
        assert_eq!(detect_language(Path::new("Dockerfile.dev")), None);
        assert_eq!(detect_language(Path::new("not-a-Dockerfile.txt")), None);
    }

    #[test]
    fn detect_unsupported() {
        assert_eq!(detect_language(Path::new("image.png")), None);
        assert_eq!(detect_language(Path::new("build.gradle")), None);
        assert_eq!(detect_language(Path::new("file.lock")), None);
        assert_eq!(detect_language(Path::new("binary.exe")), None);
    }

    #[test]
    fn walk_finds_cs_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Foo.cs"), "class Foo {}").unwrap();
        // readme.md is now indexed as "markdown" — verify both are found.
        fs::write(dir.path().join("readme.md"), "# readme").unwrap();

        let files = walk(dir.path()).unwrap();
        let cs_files: Vec<_> = files.iter().filter(|f| f.language == "csharp").collect();
        let md_files: Vec<_> = files.iter().filter(|f| f.language == "markdown").collect();
        assert_eq!(cs_files.len(), 1, "expected exactly one .cs file");
        assert!(cs_files[0].relative_path.ends_with("Foo.cs"));
        assert_eq!(md_files.len(), 1, "expected exactly one .md file");
    }

    #[test]
    fn walk_respects_gitignore() {
        let dir = TempDir::new().unwrap();

        // Create a .gitignore that excludes `generated/`
        fs::write(dir.path().join(".gitignore"), "generated/\n").unwrap();

        // File inside ignored dir — should be excluded.
        let gen_dir = dir.path().join("generated");
        fs::create_dir(&gen_dir).unwrap();
        fs::write(gen_dir.join("Auto.cs"), "// auto-generated").unwrap();

        // File in root — should be included.
        fs::write(dir.path().join("Main.cs"), "class Main {}").unwrap();

        // Initialise a git repo so .gitignore is activated.
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .output()
            .ok();

        let files = walk(dir.path()).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        assert!(paths.iter().any(|p| p.ends_with("Main.cs")), "Main.cs missing: {paths:?}");
        assert!(
            !paths.iter().any(|p| p.contains("Auto.cs")),
            "Auto.cs should be gitignored: {paths:?}"
        );
    }

    #[test]
    fn walk_result_is_sorted() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("z.cs"), "").unwrap();
        fs::write(dir.path().join("a.cs"), "").unwrap();
        fs::write(dir.path().join("m.cs"), "").unwrap();

        let files = walk(dir.path()).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "walk result should be sorted");
    }
}
