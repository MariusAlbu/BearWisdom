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
    // Languages added to bearwisdom-profile registry.
    assert_eq!(detect_language(Path::new("Main.scala")), Some("scala"));
    assert_eq!(detect_language(Path::new("main.dart")), Some("dart"));
    assert_eq!(detect_language(Path::new("lib.ex")), Some("elixir"));
    assert_eq!(detect_language(Path::new("script.exs")), Some("elixir"));
    assert_eq!(detect_language(Path::new("init.lua")), Some("lua"));
    assert_eq!(detect_language(Path::new("Main.hs")), Some("haskell"));
    // R language — registered in bearwisdom-profile.
    assert_eq!(detect_language(Path::new("analysis.r")), Some("r"));
    assert_eq!(detect_language(Path::new("Analysis.R")), Some("r"));
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
fn detect_new_languages() {
    // PowerShell
    assert_eq!(detect_language(Path::new("script.ps1")), Some("powershell"));
    assert_eq!(detect_language(Path::new("module.psm1")), Some("powershell"));
    assert_eq!(detect_language(Path::new("manifest.psd1")), Some("powershell"));
    // Groovy
    assert_eq!(detect_language(Path::new("build.gradle")), Some("groovy"));
    assert_eq!(detect_language(Path::new("Foo.groovy")), Some("groovy"));
    // Erlang
    assert_eq!(detect_language(Path::new("server.erl")), Some("erlang"));
    assert_eq!(detect_language(Path::new("header.hrl")), Some("erlang"));
    // F#
    assert_eq!(detect_language(Path::new("Main.fs")), Some("fsharp"));
    assert_eq!(detect_language(Path::new("Sig.fsi")), Some("fsharp"));
    assert_eq!(detect_language(Path::new("script.fsx")), Some("fsharp"));
    // GDScript
    assert_eq!(detect_language(Path::new("player.gd")), Some("gdscript"));
    // VB.NET
    assert_eq!(detect_language(Path::new("Module.vb")), Some("vbnet"));
    // Nim
    assert_eq!(detect_language(Path::new("app.nim")), Some("nim"));
    assert_eq!(detect_language(Path::new("config.nims")), Some("nim"));
    // Gleam
    assert_eq!(detect_language(Path::new("main.gleam")), Some("gleam"));
    // Nix
    assert_eq!(detect_language(Path::new("default.nix")), Some("nix"));
    // HCL / Terraform
    assert_eq!(detect_language(Path::new("main.tf")), Some("hcl"));
    assert_eq!(detect_language(Path::new("vars.tfvars")), Some("hcl"));
    assert_eq!(detect_language(Path::new("config.hcl")), Some("hcl"));
    // Puppet
    assert_eq!(detect_language(Path::new("nginx.pp")), Some("puppet"));
    // Starlark / Bazel
    assert_eq!(detect_language(Path::new("rules.bzl")), Some("starlark"));
    assert_eq!(detect_language(Path::new("defs.star")), Some("starlark"));
    // Protocol Buffers
    assert_eq!(detect_language(Path::new("service.proto")), Some("proto"));
    // GraphQL
    assert_eq!(detect_language(Path::new("schema.graphql")), Some("graphql"));
    assert_eq!(detect_language(Path::new("query.gql")), Some("graphql"));
    // Prisma
    assert_eq!(detect_language(Path::new("schema.prisma")), Some("prisma"));
    // Bicep
    assert_eq!(detect_language(Path::new("main.bicep")), Some("bicep"));
    // CMake
    assert_eq!(detect_language(Path::new("module.cmake")), Some("cmake"));
    // Ada
    assert_eq!(detect_language(Path::new("main.adb")), Some("ada"));
    assert_eq!(detect_language(Path::new("spec.ads")), Some("ada"));
    // Fortran
    assert_eq!(detect_language(Path::new("solver.f90")), Some("fortran"));
    assert_eq!(detect_language(Path::new("math.f95")), Some("fortran"));
    // Pascal (.pp goes to puppet; .pas and .dpr are unambiguous)
    assert_eq!(detect_language(Path::new("program.pas")), Some("pascal"));
    assert_eq!(detect_language(Path::new("project.dpr")), Some("pascal"));
    // COBOL
    assert_eq!(detect_language(Path::new("payroll.cob")), Some("cobol"));
    assert_eq!(detect_language(Path::new("report.cbl")), Some("cobol"));
    // Clojure
    assert_eq!(detect_language(Path::new("core.clj")), Some("clojure"));
    assert_eq!(detect_language(Path::new("app.cljs")), Some("clojure"));
    assert_eq!(detect_language(Path::new("shared.cljc")), Some("clojure"));
    // OCaml
    assert_eq!(detect_language(Path::new("main.ml")), Some("ocaml"));
    assert_eq!(detect_language(Path::new("sig.mli")), Some("ocaml"));
    // Svelte
    assert_eq!(detect_language(Path::new("App.svelte")), Some("svelte"));
    // Astro
    assert_eq!(detect_language(Path::new("index.astro")), Some("astro"));
    // Perl — both .pl and .pm are detected. Perl appears before Prolog in the
    // registry so .pl is claimed by Perl by default; content-based override
    // promotes Prolog when the file head shows clear Prolog markers
    // (`:- module(...)`, etc.) — see `detect_pl_content_promotes_prolog`.
    assert_eq!(detect_language(Path::new("lib.pm")), Some("perl"));
    assert_eq!(detect_language(Path::new("script.pl")), Some("perl"));
    // MATLAB profile now claims .m; this is an intentional trade-off over ObjC ambiguity
    assert_eq!(detect_language(Path::new("main.m")), Some("matlab"));
}

#[test]
fn detect_pl_content_promotes_prolog() {
    let dir = TempDir::new().unwrap();
    let pl = dir.path().join("lists.pl");
    fs::write(
        &pl,
        ":- module(lists, [member/2, append/3]).\n\nmember(X, [X | _]).\nmember(X, [_ | T]) :- member(X, T).\n",
    )
    .unwrap();
    assert_eq!(detect_language(&pl), Some("prolog"));
}

#[test]
fn detect_pl_content_keeps_perl_for_perl_script() {
    let dir = TempDir::new().unwrap();
    let pl = dir.path().join("script.pl");
    fs::write(&pl, "#!/usr/bin/perl\nuse strict;\nuse warnings;\nmy $name = 'world';\nprint \"hello $name\\n\";\n").unwrap();
    assert_eq!(detect_language(&pl), Some("perl"));
}

#[test]
fn detect_pl_content_promotes_xsb_dialect() {
    // SWI-Prolog's library/dialect/xsb/source.pl uses module + table.
    let dir = TempDir::new().unwrap();
    let pl = dir.path().join("source.pl");
    fs::write(
        &pl,
        ":- module(source, [tnot/1, get_residual/2]).\n:- table tnot/1.\n",
    )
    .unwrap();
    assert_eq!(detect_language(&pl), Some("prolog"));
}

#[test]
fn detect_pl_clause_only_promotes_prolog() {
    // No directive; just clause heads with `:-` body. Common in older
    // .P / .pl files.
    let dir = TempDir::new().unwrap();
    let pl = dir.path().join("rules.pl");
    fs::write(
        &pl,
        "% rules file\nparent(tom, bob).\nparent(tom, liz).\n\ngrandparent(X, Z) :- parent(X, Y), parent(Y, Z).\n",
    )
    .unwrap();
    assert_eq!(detect_language(&pl), Some("prolog"));
}

#[test]
fn detect_css_with_mixin_promotes_scss() {
    let dir = TempDir::new().unwrap();
    let css = dir.path().join("_mixins.css");
    fs::write(&css, "@mixin panel($width) {\n  max-width: $width;\n}\n").unwrap();
    assert_eq!(detect_language(&css), Some("scss"));
}

#[test]
fn detect_css_with_include_promotes_scss() {
    let dir = TempDir::new().unwrap();
    let css = dir.path().join("styles.css");
    fs::write(&css, ".btn {\n  @include rounded(4px);\n}\n").unwrap();
    assert_eq!(detect_language(&css), Some("scss"));
}

#[test]
fn detect_plain_css_stays_css() {
    let dir = TempDir::new().unwrap();
    let css = dir.path().join("styles.css");
    fs::write(&css, ".btn {\n  color: red;\n  display: inline-block;\n}\n").unwrap();
    assert_eq!(detect_language(&css), Some("css"));
}

#[test]
fn detect_unsupported() {
    assert_eq!(detect_language(Path::new("image.png")), None);
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
