//! Integration test for M1 — per-package manifest reading.
//!
//! Verifies that `read_all_manifests_per_package` returns one entry per
//! workspace package for realistic monorepo layouts (pnpm, Cargo workspace,
//! Gradle multi-module, .NET solution, Mix umbrella) and that each entry's
//! dep set is isolated from siblings.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use bearwisdom::ecosystem::manifest::{
    read_all_manifests, read_all_manifests_per_package, ManifestKind, PackageManifest,
};
use tempfile::TempDir;

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

fn names_of<'a>(
    manifests: &'a [PackageManifest],
    kind: ManifestKind,
) -> HashSet<&'a str> {
    manifests
        .iter()
        .filter(|m| m.kind == kind)
        .map(|m| m.name.as_str())
        .collect()
}

/// pnpm-style monorepo modeled on ts-immich's layout:
/// root + 9 workspace packages under different directory shapes.
#[test]
fn pnpm_9_package_monorepo() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Root manifest — declares workspaces, no deps of its own.
    write_file(
        root,
        "package.json",
        r#"{"name":"immich","private":true,"workspaces":["server","web","cli","e2e","sdk","docs","open-api/typescript-sdk","mobile","machine-learning-js"]}"#,
    );

    // 9 workspace packages, each with its own distinct deps.
    write_file(
        root,
        "server/package.json",
        r#"{"name":"@immich/server","dependencies":{"@nestjs/core":"10","typeorm":"0.3","bcrypt":"5"}}"#,
    );
    write_file(
        root,
        "web/package.json",
        r#"{"name":"@immich/web","dependencies":{"svelte":"4","vite":"5","tailwindcss":"3"}}"#,
    );
    write_file(
        root,
        "cli/package.json",
        r#"{"name":"@immich/cli","dependencies":{"commander":"11","chalk":"5"}}"#,
    );
    write_file(
        root,
        "e2e/package.json",
        r#"{"name":"@immich/e2e","devDependencies":{"@playwright/test":"1.40","vitest":"1"}}"#,
    );
    write_file(
        root,
        "sdk/package.json",
        r#"{"name":"@immich/sdk","dependencies":{"axios":"1"}}"#,
    );
    write_file(
        root,
        "docs/package.json",
        r#"{"name":"@immich/docs","dependencies":{"@docusaurus/core":"3","react":"18"}}"#,
    );
    write_file(
        root,
        "open-api/typescript-sdk/package.json",
        r#"{"name":"@immich/typescript-sdk","dependencies":{"axios":"1"}}"#,
    );
    write_file(
        root,
        "mobile/package.json",
        r#"{"name":"@immich/mobile","dependencies":{"expo":"50"}}"#,
    );
    write_file(
        root,
        "machine-learning-js/package.json",
        r#"{"name":"@immich/ml-js","dependencies":{"onnxruntime-node":"1"}}"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let npm: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Npm)
        .collect();

    // Root + 9 workspace packages = 10 entries.
    assert_eq!(
        npm.len(),
        10,
        "expected 10 npm manifests, got {}",
        npm.len()
    );

    let names = names_of(&per_pkg, ManifestKind::Npm);
    let expected: [&str; 10] = [
        "immich",
        "@immich/server",
        "@immich/web",
        "@immich/cli",
        "@immich/e2e",
        "@immich/sdk",
        "@immich/docs",
        "@immich/typescript-sdk",
        "@immich/mobile",
        "@immich/ml-js",
    ];
    for n in expected {
        assert!(names.contains(n), "missing package '{n}'");
    }

    // Per-package dep isolation: server has @nestjs/core but NOT svelte or
    // @playwright/test. This is the exact invariant M2 needs for correct
    // per-package classification — server/ code must NOT classify Playwright
    // as external just because e2e/ declares it.
    let server = npm.iter().find(|m| m.name == "@immich/server").unwrap();
    assert!(server.data.dependencies.contains("@nestjs/core"));
    assert!(server.data.dependencies.contains("typeorm"));
    assert!(!server.data.dependencies.contains("svelte"));
    assert!(!server.data.dependencies.contains("@playwright/test"));

    let web = npm.iter().find(|m| m.name == "@immich/web").unwrap();
    assert!(web.data.dependencies.contains("svelte"));
    assert!(!web.data.dependencies.contains("@nestjs/core"));

    let e2e = npm.iter().find(|m| m.name == "@immich/e2e").unwrap();
    assert!(e2e.data.dependencies.contains("@playwright/test"));
    assert!(!e2e.data.dependencies.contains("@nestjs/core"));
    assert!(!e2e.data.dependencies.contains("svelte"));

    // Legacy read_all_manifests unions everything — useful sanity check.
    let legacy = read_all_manifests(root);
    let legacy_npm = legacy.get(&ManifestKind::Npm).unwrap();
    assert!(legacy_npm.dependencies.contains("@nestjs/core"));
    assert!(legacy_npm.dependencies.contains("svelte"));
    assert!(legacy_npm.dependencies.contains("@playwright/test"));
}

/// Cargo workspace with 3 member crates, modeled on rust-lemmy.
#[test]
fn cargo_workspace_with_members() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Workspace root — no [package] section, only [workspace].
    write_file(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/api\", \"crates/db\", \"crates/utils\"]\n",
    );

    write_file(
        root,
        "crates/api/Cargo.toml",
        "[package]\nname = \"lemmy-api\"\nversion = \"0.1.0\"\n\n[dependencies]\nactix-web = \"4\"\nserde = \"1\"\n",
    );
    write_file(
        root,
        "crates/db/Cargo.toml",
        "[package]\nname = \"lemmy-db\"\nversion = \"0.1.0\"\n\n[dependencies]\ndiesel = \"2\"\nserde = \"1\"\n",
    );
    write_file(
        root,
        "crates/utils/Cargo.toml",
        "[package]\nname = \"lemmy-utils\"\nversion = \"0.1.0\"\n\n[dependencies]\ntracing = \"0.1\"\n",
    );

    let per_pkg = read_all_manifests_per_package(root);
    let cargo: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Cargo)
        .collect();

    // Workspace root + 3 members.
    assert_eq!(cargo.len(), 4);

    let names = names_of(&per_pkg, ManifestKind::Cargo);
    assert!(names.contains("lemmy-api"));
    assert!(names.contains("lemmy-db"));
    assert!(names.contains("lemmy-utils"));

    // Per-member dep isolation.
    let api = cargo.iter().find(|m| m.name == "lemmy-api").unwrap();
    assert!(api.data.dependencies.contains("actix-web"));
    assert!(!api.data.dependencies.contains("diesel"));
    assert!(!api.data.dependencies.contains("tracing"));

    let db = cargo.iter().find(|m| m.name == "lemmy-db").unwrap();
    assert!(db.data.dependencies.contains("diesel"));
    assert!(!db.data.dependencies.contains("actix-web"));

    let utils = cargo.iter().find(|m| m.name == "lemmy-utils").unwrap();
    assert!(utils.data.dependencies.contains("tracing"));
    assert!(!utils.data.dependencies.contains("serde"));
}

/// .NET solution with 3 csproj files, modeled on eShop.
#[test]
fn dotnet_solution_multi_csproj() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "src/WebApi/WebApi.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk.Web">
    <ItemGroup>
        <PackageReference Include="Microsoft.EntityFrameworkCore" Version="8.0.0" />
        <PackageReference Include="Serilog.AspNetCore" Version="8.0.0" />
    </ItemGroup>
</Project>"#,
    );
    write_file(
        root,
        "src/Worker/Worker.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk.Worker">
    <ItemGroup>
        <PackageReference Include="MassTransit" Version="8.0.0" />
    </ItemGroup>
</Project>"#,
    );
    write_file(
        root,
        "tests/UnitTests/UnitTests.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk">
    <ItemGroup>
        <PackageReference Include="xunit" Version="2.5.0" />
    </ItemGroup>
</Project>"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let csproj: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::NuGet)
        .collect();

    assert_eq!(csproj.len(), 3);

    let names = names_of(&per_pkg, ManifestKind::NuGet);
    assert!(names.contains("WebApi"));
    assert!(names.contains("Worker"));
    assert!(names.contains("UnitTests"));

    // Per-project SDK type is distinct — each project declares its own.
    let web = csproj.iter().find(|m| m.name == "WebApi").unwrap();
    assert_eq!(web.data.sdk_type.as_deref(), Some("web"));
    assert!(web.data.dependencies.contains("Microsoft.EntityFrameworkCore"));
    assert!(!web.data.dependencies.contains("MassTransit"));
    assert!(!web.data.dependencies.contains("xunit"));

    let worker = csproj.iter().find(|m| m.name == "Worker").unwrap();
    assert_eq!(worker.data.sdk_type.as_deref(), Some("worker"));
    assert!(worker.data.dependencies.contains("MassTransit"));

    let tests = csproj.iter().find(|m| m.name == "UnitTests").unwrap();
    assert_eq!(tests.data.sdk_type.as_deref(), Some("base"));
    assert!(tests.data.dependencies.contains("xunit"));
}

/// Gradle multi-module build with root + 2 modules.
#[test]
fn gradle_multi_module() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "build.gradle",
        "plugins { id 'java' }\n", // root, no deps
    );
    write_file(
        root,
        "settings.gradle",
        "rootProject.name = 'app'\ninclude 'core', 'web'\n",
    );
    write_file(
        root,
        "core/build.gradle",
        r#"dependencies {
    implementation 'com.google.guava:guava:33.0.0-jre'
}
"#,
    );
    write_file(
        root,
        "web/build.gradle",
        r#"dependencies {
    implementation 'org.springframework.boot:spring-boot-starter-web:3.2.0'
}
"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let gradle: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Gradle)
        .collect();

    assert_eq!(gradle.len(), 3);

    let core = gradle.iter().find(|m| m.name == "core").unwrap();
    assert!(core.data.dependencies.contains("com.google.guava"));
    assert!(!core.data.dependencies.contains("org.springframework.boot"));

    let web = gradle.iter().find(|m| m.name == "web").unwrap();
    assert!(web.data.dependencies.contains("org.springframework.boot"));
    assert!(!web.data.dependencies.contains("com.google.guava"));
}

/// Elixir mix umbrella with 2 apps.
#[test]
fn mix_umbrella_apps() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "mix.exs",
        r#"defmodule Umbrella.MixProject do
  use Mix.Project
  def project do
    [
      app: :umbrella,
      apps_path: "apps"
    ]
  end
  defp deps do
    []
  end
end
"#,
    );
    write_file(
        root,
        "apps/web/mix.exs",
        r#"defmodule Web.MixProject do
  use Mix.Project
  def project do
    [
      app: :web,
      deps: deps()
    ]
  end
  defp deps do
    [
      {:phoenix, "~> 1.7"},
      {:phoenix_live_view, "~> 0.20"},
    ]
  end
end
"#,
    );
    write_file(
        root,
        "apps/core/mix.exs",
        r#"defmodule Core.MixProject do
  use Mix.Project
  def project do
    [
      app: :core,
      deps: deps()
    ]
  end
  defp deps do
    [
      {:ecto_sql, "~> 3.10"},
      {:postgrex, ">= 0.0.0"},
    ]
  end
end
"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let mix: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Mix)
        .collect();

    assert_eq!(mix.len(), 3);

    let names = names_of(&per_pkg, ManifestKind::Mix);
    assert!(names.contains("umbrella"));
    assert!(names.contains("web"));
    assert!(names.contains("core"));

    let web = mix.iter().find(|m| m.name == "web").unwrap();
    assert!(web.data.dependencies.contains("phoenix"));
    assert!(!web.data.dependencies.contains("ecto_sql"));

    let core = mix.iter().find(|m| m.name == "core").unwrap();
    assert!(core.data.dependencies.contains("ecto_sql"));
    assert!(!core.data.dependencies.contains("phoenix"));
}

/// Dart/Flutter melos-style monorepo with 2 packages.
#[test]
fn pubspec_multi_package() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "pubspec.yaml",
        "name: root_workspace\n",
    );
    write_file(
        root,
        "packages/app/pubspec.yaml",
        r#"name: my_app
version: 1.0.0

dependencies:
  flutter:
    sdk: flutter
  http: ^0.13.0
"#,
    );
    write_file(
        root,
        "packages/core/pubspec.yaml",
        r#"name: my_core
version: 1.0.0

dependencies:
  meta: ^1.9.0
"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let pubspec: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Pubspec)
        .collect();

    assert_eq!(pubspec.len(), 3);

    let app = pubspec.iter().find(|m| m.name == "my_app").unwrap();
    assert!(app.data.dependencies.contains("http"));
    assert!(!app.data.dependencies.contains("meta"));

    let core = pubspec.iter().find(|m| m.name == "my_core").unwrap();
    assert!(core.data.dependencies.contains("meta"));
    assert!(!core.data.dependencies.contains("http"));
}

/// PHP Composer monorepo (less common but valid — Drupal multi-package layouts).
#[test]
fn composer_multi_package() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "composer.json",
        r#"{"name":"vendor/root","require":{}}"#,
    );
    write_file(
        root,
        "packages/api/composer.json",
        r#"{"name":"vendor/api","require":{"symfony/http-foundation":"^6"}}"#,
    );
    write_file(
        root,
        "packages/worker/composer.json",
        r#"{"name":"vendor/worker","require":{"symfony/messenger":"^6"}}"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let composer: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Composer)
        .collect();

    assert_eq!(composer.len(), 3);

    let api = composer.iter().find(|m| m.name == "vendor/api").unwrap();
    assert!(api.data.dependencies.contains("symfony/http-foundation"));
    assert!(!api.data.dependencies.contains("symfony/messenger"));
}

/// Single-package project — the common case. Verifies M1 doesn't regress it.
#[test]
fn single_package_project() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "package.json",
        r#"{"name":"solo","dependencies":{"lodash":"4","react":"18"}}"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let npm: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Npm)
        .collect();

    assert_eq!(npm.len(), 1);
    assert_eq!(npm[0].name, "solo");
    assert!(npm[0].path.as_os_str().is_empty());
    assert!(npm[0].data.dependencies.contains("lodash"));
    assert!(npm[0].data.dependencies.contains("react"));

    // Node builtins are always present in the union (pre-M1 behavior preserved).
    assert!(npm[0].data.dependencies.contains("fs"));
    assert!(npm[0].data.dependencies.contains("http"));
}

/// Maven multi-module build with parent pom + 2 modules.
#[test]
fn maven_multi_module() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "pom.xml",
        r#"<?xml version="1.0"?>
<project>
    <modelVersion>4.0.0</modelVersion>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0.0</version>
    <packaging>pom</packaging>
    <modules>
        <module>api</module>
        <module>core</module>
    </modules>
</project>"#,
    );
    write_file(
        root,
        "api/pom.xml",
        r#"<?xml version="1.0"?>
<project>
    <parent>
        <groupId>com.example</groupId>
        <artifactId>parent</artifactId>
        <version>1.0.0</version>
    </parent>
    <artifactId>api</artifactId>
    <dependencies>
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-web</artifactId>
        </dependency>
    </dependencies>
</project>"#,
    );
    write_file(
        root,
        "core/pom.xml",
        r#"<?xml version="1.0"?>
<project>
    <parent>
        <groupId>com.example</groupId>
        <artifactId>parent</artifactId>
        <version>1.0.0</version>
    </parent>
    <artifactId>core</artifactId>
    <dependencies>
        <dependency>
            <groupId>com.google.guava</groupId>
            <artifactId>guava</artifactId>
        </dependency>
    </dependencies>
</project>"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let maven: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Maven)
        .collect();

    assert_eq!(maven.len(), 3);

    let names = names_of(&per_pkg, ManifestKind::Maven);
    assert!(names.contains("parent"));
    assert!(names.contains("api"));
    assert!(names.contains("core"));

    let api = maven.iter().find(|m| m.name == "api").unwrap();
    assert!(api.data.dependencies.contains("org.springframework.boot"));
    assert!(!api.data.dependencies.contains("com.google.guava"));

    let core = maven.iter().find(|m| m.name == "core").unwrap();
    assert!(core.data.dependencies.contains("com.google.guava"));
    assert!(!core.data.dependencies.contains("org.springframework.boot"));
}

/// Paths must be relative to project_root; manifest_path must be absolute.
#[test]
fn path_invariants() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        root,
        "apps/web/package.json",
        r#"{"name":"web","dependencies":{}}"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let web = per_pkg.iter().find(|m| m.name == "web").unwrap();

    assert!(web.path.is_relative(), "package path must be relative");
    assert!(
        web.path.starts_with("apps"),
        "package path should start with 'apps', got {:?}",
        web.path
    );
    assert!(web.manifest_path.is_absolute(), "manifest_path must be absolute");
    assert!(web.manifest_path.ends_with("package.json"));
}
