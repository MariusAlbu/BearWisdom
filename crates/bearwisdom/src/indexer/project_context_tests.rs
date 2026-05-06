use super::*;

#[test]
fn test_parse_sdk_type_web() {
    let csproj = r#"<Project Sdk="Microsoft.NET.Sdk.Web">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
</Project>"#;
    assert_eq!(parse_sdk_type(csproj), Some(DotnetSdkType::Web));
}

#[test]
fn test_parse_sdk_type_base() {
    let csproj = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
</Project>"#;
    assert_eq!(parse_sdk_type(csproj), Some(DotnetSdkType::Base));
}

#[test]
fn test_parse_package_references() {
    let csproj = r#"<Project Sdk="Microsoft.NET.Sdk.Web">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <PackageReference Include="MediatR" Version="12.0.1" />
    <PackageReference Include="Serilog.AspNetCore" Version="8.0.0" />
  </ItemGroup>
</Project>"#;
    let pkgs = parse_package_references(csproj);
    assert_eq!(pkgs, vec!["Newtonsoft.Json", "MediatR", "Serilog.AspNetCore"]);
}

#[test]
fn test_parse_package_reference_multiline() {
    let csproj = r#"<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Microsoft.EntityFrameworkCore.Design" Version="9.0.3">
      <PrivateAssets>all</PrivateAssets>
    </PackageReference>
  </ItemGroup>
</Project>"#;
    let pkgs = parse_package_references(csproj);
    assert_eq!(pkgs, vec!["Microsoft.EntityFrameworkCore.Design"]);
}

#[test]
fn test_parse_global_usings() {
    let content = r#"global using System.ComponentModel.DataAnnotations;
global using System.Security.Claims;
global using eShop.Basket.API.Extensions;
global using static System.Math;
"#;
    let usings = parse_global_usings(content);
    assert_eq!(usings, vec![
        "System.ComponentModel.DataAnnotations",
        "System.Security.Claims",
        "eShop.Basket.API.Extensions",
    ]);
}

#[test]
fn test_implicit_usings_base_sdk() {
    let usings = implicit_usings_for_sdk(DotnetSdkType::Base);
    assert!(usings.contains(&"System"));
    assert!(usings.contains(&"System.Linq"));
    assert!(usings.contains(&"System.Threading.Tasks"));
    assert!(!usings.contains(&"Microsoft.AspNetCore.Builder"));
}

#[test]
fn test_implicit_usings_web_sdk() {
    let usings = implicit_usings_for_sdk(DotnetSdkType::Web);
    assert!(usings.contains(&"System.Linq"));
    assert!(usings.contains(&"Microsoft.AspNetCore.Builder"));
    assert!(usings.contains(&"Microsoft.Extensions.Logging"));
    assert!(usings.contains(&"Microsoft.Extensions.DependencyInjection"));
}

#[test]
fn test_is_external_namespace() {
    use super::manifest::{ManifestData, ManifestKind};
    let mut ctx = ProjectContext::default();
    let mut nuget = ManifestData::default();
    nuget.dependencies.insert("Newtonsoft.Json".to_string());
    nuget.dependencies.insert("MediatR".to_string());
    ctx.manifests.insert(ManifestKind::NuGet, nuget);

    let nuget = ctx.manifest(ManifestKind::NuGet).unwrap();
    assert!(nuget.dependencies.contains("Newtonsoft.Json"));
    assert!(nuget.dependencies.contains("MediatR"));
    assert!(!nuget.dependencies.contains("App.Models"));
    assert!(!nuget.dependencies.contains("eShop.Catalog"));
    // System/Microsoft are base prefixes — never need to be in the manifest.
    assert!(!nuget.dependencies.contains("System"));
    assert!(!nuget.dependencies.contains("Microsoft"));
}

#[test]
fn test_most_capable_sdk() {
    assert_eq!(most_capable_sdk(&[DotnetSdkType::Base, DotnetSdkType::Web]), DotnetSdkType::Web);
    assert_eq!(most_capable_sdk(&[DotnetSdkType::Worker]), DotnetSdkType::Worker);
    assert_eq!(most_capable_sdk(&[]), DotnetSdkType::Other);
    assert_eq!(most_capable_sdk(&[DotnetSdkType::Base]), DotnetSdkType::Base);
}


// ===========================================================================
// M2 — per-package context lookups
// ===========================================================================

#[cfg(test)]
mod m2_tests {
    use super::super::*;
    use super::super::manifest::ManifestKind;
    use crate::types::PackageInfo;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(root: &std::path::Path, rel: &str, content: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn manifests_for_returns_per_package_when_available() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "server/package.json",
            r#"{"name":"server","dependencies":{"express":"4"}}"#,
        );
        write_file(
            root,
            "web/package.json",
            r#"{"name":"web","dependencies":{"react":"18"}}"#,
        );

        let packages = vec![
            PackageInfo {
                id: Some(1),
                name: "server".into(),
                path: "server".into(),
                kind: Some("npm".into()),
                manifest: Some("server/package.json".into()),
                declared_name: None,
            },
            PackageInfo {
                id: Some(2),
                name: "web".into(),
                path: "web".into(),
                kind: Some("npm".into()),
                manifest: Some("web/package.json".into()),
                declared_name: None,
            },
        ];

        let ctx = build_project_context_with_packages(root, &packages);
        assert!(ctx.is_per_package());

        // Package 1 (server) sees express, NOT react.
        assert!(ctx.has_dependency_for(Some(1), ManifestKind::Npm, "express"));
        assert!(!ctx.has_dependency_for(Some(1), ManifestKind::Npm, "react"));

        // Package 2 (web) sees react, NOT express.
        assert!(ctx.has_dependency_for(Some(2), ManifestKind::Npm, "react"));
        assert!(!ctx.has_dependency_for(Some(2), ManifestKind::Npm, "express"));

        // Unknown package id or None falls back to the union — both deps visible.
        assert!(ctx.has_dependency_for(None, ManifestKind::Npm, "express"));
        assert!(ctx.has_dependency_for(None, ManifestKind::Npm, "react"));
        assert!(ctx.has_dependency_for(Some(999), ManifestKind::Npm, "express"));
    }

    #[test]
    fn legacy_builder_leaves_by_package_empty() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "package.json",
            r#"{"name":"solo","dependencies":{"lodash":"4"}}"#,
        );

        let ctx = build_project_context(root);
        assert!(!ctx.is_per_package());
        // has_dependency_for with any id falls back to the union.
        assert!(ctx.has_dependency_for(Some(1), ManifestKind::Npm, "lodash"));
    }

    #[test]
    fn single_package_with_packages_populates_by_package() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "package.json",
            r#"{"name":"solo","dependencies":{"axios":"1"}}"#,
        );

        // Even single-package mode gets a per-package entry at path "".
        let packages = vec![PackageInfo {
            id: Some(1),
            name: "solo".into(),
            path: "".into(),
            kind: Some("npm".into()),
            manifest: Some("package.json".into()),
            declared_name: None,
        }];

        let ctx = build_project_context_with_packages(root, &packages);
        assert!(ctx.is_per_package());
        assert!(ctx.has_dependency_for(Some(1), ManifestKind::Npm, "axios"));
    }

    #[test]
    fn package_with_no_manifest_yields_no_entry() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "server/package.json",
            r#"{"name":"server","dependencies":{"express":"4"}}"#,
        );
        // packages/empty has NO manifest — should get no by_package entry.

        let packages = vec![
            PackageInfo {
                id: Some(1),
                name: "server".into(),
                path: "server".into(),
                kind: Some("npm".into()),
                manifest: Some("server/package.json".into()),
                declared_name: None,
            },
            PackageInfo {
                id: Some(2),
                name: "empty".into(),
                path: "packages/empty".into(),
                kind: None,
                manifest: None,
                declared_name: None,
            },
        ];

        let ctx = build_project_context_with_packages(root, &packages);
        assert!(ctx.by_package.contains_key(&1));
        assert!(!ctx.by_package.contains_key(&2));
        // Package 2 (empty) falls back to union — sees express.
        assert!(ctx.has_dependency_for(Some(2), ManifestKind::Npm, "express"));
    }
}

// ---------------------------------------------------------------------------
// Activation-evaluator tests — ManifestMatch + ManifestFieldContains
// ---------------------------------------------------------------------------

#[cfg(test)]
mod activation_eval_tests {
    use super::*;
    use crate::ecosystem::{EcosystemActivation, EcosystemId};
    use crate::ecosystem::manifest::{ManifestData, ManifestKind};
    use std::fs;

    fn ctx_with_manifests(kinds: &[ManifestKind]) -> ProjectContext {
        let mut ctx = ProjectContext::default();
        for k in kinds {
            ctx.manifests.insert(*k, ManifestData::default());
        }
        ctx
    }

    fn ctx_with_root(root: &std::path::Path) -> ProjectContext {
        let mut ctx = ProjectContext::default();
        ctx.project_root = root.to_path_buf();
        ctx
    }

    #[test]
    fn manifest_match_fires_when_eco_specific_manifest_present() {
        let ctx = ctx_with_manifests(&[ManifestKind::Cargo]);
        let cargo_id = EcosystemId::new("cargo");
        assert!(super::evaluate_activation(
            &EcosystemActivation::ManifestMatch,
            cargo_id,
            &ctx,
            &[],
        ));
    }

    #[test]
    fn manifest_match_does_not_fire_for_unrelated_manifest() {
        // Project has Cargo.toml. Asking npm to fire on ManifestMatch must
        // return false — npm's manifest kind (Npm) isn't present.
        let ctx = ctx_with_manifests(&[ManifestKind::Cargo]);
        let npm_id = EcosystemId::new("npm");
        assert!(!super::evaluate_activation(
            &EcosystemActivation::ManifestMatch,
            npm_id,
            &ctx,
            &[],
        ));
    }

    #[test]
    fn manifest_match_does_not_fire_for_ecosystem_with_no_kinds() {
        // Ecosystems not in `manifest_kinds_for_ecosystem` (cabal, nimble,
        // cpan, all stdlib) must rely on a sibling `LanguagePresent` clause;
        // pure `ManifestMatch` must evaluate to false.
        let ctx = ctx_with_manifests(&[ManifestKind::Cargo, ManifestKind::Npm]);
        let unknown = EcosystemId::new("cabal");
        assert!(!super::evaluate_activation(
            &EcosystemActivation::ManifestMatch,
            unknown,
            &ctx,
            &[],
        ));
    }

    #[test]
    fn manifest_match_unions_multiple_kinds() {
        // The maven ecosystem claims Maven, Gradle, Sbt, Clojure. Any one
        // satisfies ManifestMatch.
        let maven = EcosystemId::new("maven");
        for kind in [ManifestKind::Maven, ManifestKind::Gradle, ManifestKind::Sbt, ManifestKind::Clojure] {
            let ctx = ctx_with_manifests(&[kind]);
            assert!(
                super::evaluate_activation(&EcosystemActivation::ManifestMatch, maven, &ctx, &[]),
                "maven should fire for {kind:?}",
            );
        }
    }

    #[test]
    fn field_contains_json_array_membership_case_insensitive() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("tsconfig.json"),
            r#"{ "compilerOptions": { "lib": ["DOM", "ES2022"] } }"#,
        )
        .unwrap();
        let ctx = ctx_with_root(dir.path());
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/tsconfig.json",
            field_path: "compilerOptions.lib",
            value: "dom",
        };
        assert!(super::evaluate_activation(&act, EcosystemId::new("ts-lib-dom"), &ctx, &[]));
    }

    #[test]
    fn field_contains_json_misses_when_value_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("tsconfig.json"),
            r#"{ "compilerOptions": { "lib": ["ES2022"] } }"#,
        )
        .unwrap();
        let ctx = ctx_with_root(dir.path());
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/tsconfig.json",
            field_path: "compilerOptions.lib",
            value: "DOM",
        };
        assert!(!super::evaluate_activation(&act, EcosystemId::new("ts-lib-dom"), &ctx, &[]));
    }

    #[test]
    fn field_contains_json_misses_when_field_path_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("tsconfig.json"),
            r#"{ "compilerOptions": { "target": "es2022" } }"#,
        )
        .unwrap();
        let ctx = ctx_with_root(dir.path());
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/tsconfig.json",
            field_path: "compilerOptions.lib",
            value: "DOM",
        };
        assert!(!super::evaluate_activation(&act, EcosystemId::new("ts-lib-dom"), &ctx, &[]));
    }

    #[test]
    fn field_contains_yaml_map_key_present() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("pubspec.yaml"),
            "name: my_app\ndependencies:\n  flutter:\n    sdk: flutter\n  http: ^1.0.0\n",
        )
        .unwrap();
        let ctx = ctx_with_root(dir.path());
        // For YAML maps, "contains" tests key presence — flutter is a key
        // under dependencies.
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/pubspec.yaml",
            field_path: "dependencies",
            value: "flutter",
        };
        assert!(super::evaluate_activation(&act, EcosystemId::new("flutter-sdk"), &ctx, &[]));
    }

    #[test]
    fn field_contains_yaml_misses_unrelated_key() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("pubspec.yaml"),
            "name: pure_dart\ndependencies:\n  http: ^1.0.0\n",
        )
        .unwrap();
        let ctx = ctx_with_root(dir.path());
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/pubspec.yaml",
            field_path: "dependencies",
            value: "flutter",
        };
        assert!(!super::evaluate_activation(&act, EcosystemId::new("flutter-sdk"), &ctx, &[]));
    }

    #[test]
    fn field_contains_handles_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let ctx = ctx_with_root(dir.path());
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/tsconfig.json",
            field_path: "compilerOptions.lib",
            value: "DOM",
        };
        assert!(!super::evaluate_activation(&act, EcosystemId::new("ts-lib-dom"), &ctx, &[]));
    }

    #[test]
    fn field_contains_returns_false_when_project_root_empty() {
        // ProjectContext::default has empty project_root — must not panic
        // and must evaluate to false.
        let ctx = ProjectContext::default();
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/tsconfig.json",
            field_path: "compilerOptions.lib",
            value: "DOM",
        };
        assert!(!super::evaluate_activation(&act, EcosystemId::new("ts-lib-dom"), &ctx, &[]));
    }

    #[test]
    fn field_contains_finds_nested_manifest() {
        let dir = tempfile::TempDir::new().unwrap();
        let nested = dir.path().join("packages").join("web");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("tsconfig.json"),
            r#"{ "compilerOptions": { "lib": ["DOM"] } }"#,
        )
        .unwrap();
        let ctx = ctx_with_root(dir.path());
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/tsconfig.json",
            field_path: "compilerOptions.lib",
            value: "DOM",
        };
        assert!(super::evaluate_activation(&act, EcosystemId::new("ts-lib-dom"), &ctx, &[]));
    }

    #[test]
    fn field_contains_plain_text_fallback_for_unknown_extension() {
        // project.godot is INI-shaped, not JSON/YAML. The fallback does a
        // substring search on the file content.
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("project.godot"),
            "[application]\nconfig/name=\"My Game\"\nrun/main_scene=\"res://Main.tscn\"\n",
        )
        .unwrap();
        let ctx = ctx_with_root(dir.path());
        let act = EcosystemActivation::ManifestFieldContains {
            manifest_glob: "**/project.godot",
            field_path: "application",
            value: "config/name",
        };
        assert!(super::evaluate_activation(&act, EcosystemId::new("godot-api"), &ctx, &[]));
    }
}

// ---------------------------------------------------------------------------
// Per-package activation — Phase 1 (decision-2026-05-06-r87)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod per_package_activation_tests {
    use super::*;
    use crate::ecosystem::{self, EcosystemId};
    use std::fs;
    use tempfile::TempDir;

    fn write_file(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    /// Polyglot monorepo: frontend declares `compilerOptions.lib: ["DOM"]`
    /// in its own tsconfig; backend's tsconfig targets Node-only with no
    /// DOM entry. Workspace-flat activation activated `ts-lib-dom` for both.
    /// Per-package activation must isolate the activation to the frontend.
    #[test]
    fn per_package_activation_isolates_ts_lib_dom_to_dom_package() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(
            root,
            "apps/web/tsconfig.json",
            r#"{"compilerOptions":{"target":"es2020","lib":["DOM","ES2020"]}}"#,
        );
        write_file(root, "apps/web/package.json", r#"{"name":"web"}"#);

        write_file(
            root,
            "services/api/tsconfig.json",
            r#"{"compilerOptions":{"target":"es2020","lib":["ES2020"]}}"#,
        );
        write_file(root, "services/api/package.json", r#"{"name":"api"}"#);

        let packages = vec![
            PackageInfo {
                id: Some(1),
                name: "web".into(),
                path: "apps/web".into(),
                kind: Some("npm".into()),
                manifest: Some("apps/web/package.json".into()),
                declared_name: None,
            },
            PackageInfo {
                id: Some(2),
                name: "api".into(),
                path: "services/api".into(),
                kind: Some("npm".into()),
                manifest: Some("services/api/package.json".into()),
                declared_name: None,
            },
        ];

        let ctx = build_project_context_with_packages(root, &packages);
        let registry = ecosystem::default_registry();
        let per_pkg =
            super::evaluate_active_ecosystems_per_package(&ctx, registry, &packages);

        let ts_lib_dom = EcosystemId::new("ts-lib-dom");
        let web_actives = per_pkg.get(&1).expect("web package must be evaluated");
        let api_actives = per_pkg.get(&2).expect("api package must be evaluated");

        assert!(
            web_actives.contains(&ts_lib_dom),
            "web (DOM in lib) must activate ts-lib-dom: got {web_actives:?}"
        );
        assert!(
            !api_actives.contains(&ts_lib_dom),
            "api (no DOM in lib) must NOT activate ts-lib-dom: got {api_actives:?}"
        );
    }

    /// `ProjectContext::initialize` populates both the per-package map AND
    /// derives `active_ecosystems` as the union, so workspace-wide consumers
    /// keep observing every ecosystem at least one package activated.
    #[test]
    fn initialize_unions_per_package_actives_into_workspace_wide_set() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(
            root,
            "apps/web/tsconfig.json",
            r#"{"compilerOptions":{"lib":["DOM"]}}"#,
        );
        write_file(root, "apps/web/package.json", r#"{"name":"web"}"#);
        write_file(root, "services/api/package.json", r#"{"name":"api"}"#);

        let packages = vec![
            PackageInfo {
                id: Some(1),
                name: "web".into(),
                path: "apps/web".into(),
                kind: Some("npm".into()),
                manifest: Some("apps/web/package.json".into()),
                declared_name: None,
            },
            PackageInfo {
                id: Some(2),
                name: "api".into(),
                path: "services/api".into(),
                kind: Some("npm".into()),
                manifest: Some("services/api/package.json".into()),
                declared_name: None,
            },
        ];

        let registry = ecosystem::default_registry();
        let ctx =
            ProjectContext::initialize(root, &packages, std::iter::empty::<String>(), registry);

        let ts_lib_dom = EcosystemId::new("ts-lib-dom");
        assert!(
            ctx.active_ecosystems.contains(&ts_lib_dom),
            "workspace-wide actives must union per-package actives"
        );
        assert!(ctx.active_ecosystems_by_package.contains_key(&1));
        assert!(ctx.active_ecosystems_by_package.contains_key(&2));
        assert!(ctx.active_ecosystems_by_package[&1].contains(&ts_lib_dom));
        assert!(!ctx.active_ecosystems_by_package[&2].contains(&ts_lib_dom));
    }

    /// Single-project layouts (empty `packages` slice) skip the per-package
    /// path entirely — `active_ecosystems_by_package` stays empty and the
    /// workspace-wide evaluator drives `active_ecosystems` exactly as before.
    #[test]
    fn initialize_skips_per_package_when_packages_slice_empty() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "tsconfig.json",
            r#"{"compilerOptions":{"lib":["DOM"]}}"#,
        );

        let registry = ecosystem::default_registry();
        let ctx =
            ProjectContext::initialize(root, &[], std::iter::empty::<String>(), registry);

        assert!(ctx.active_ecosystems_by_package.is_empty());
        let ts_lib_dom = EcosystemId::new("ts-lib-dom");
        assert!(
            ctx.active_ecosystems.contains(&ts_lib_dom),
            "single-project workspace-wide path must still activate ts-lib-dom on root tsconfig"
        );
    }

    /// Phase 5: per-package `LanguagePresent` narrows correctly.
    /// Polyglot monorepo with one Kotlin package and one Python package.
    /// Without per-package language presence, kotlin-stdlib activates
    /// for both packages because workspace-wide language_presence
    /// contains "kotlin". With per-package language presence, only the
    /// Kotlin package activates kotlin-stdlib.
    #[test]
    fn per_package_language_presence_narrows_kotlin_stdlib() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Synthetic on-disk layout: each package has at least one file
        // in its own language. The activation evaluator only consults
        // the language map; manifests are absent so manifest-driven
        // activations don't fire — clean isolation of the language
        // signal under test.
        write_file(root, "apps/jvm/Main.kt", "fun main() {}\n");
        write_file(root, "services/py/app.py", "def main(): pass\n");

        let packages = vec![
            PackageInfo {
                id: Some(1),
                name: "jvm".into(),
                path: "apps/jvm".into(),
                kind: Some("kotlin".into()),
                manifest: None,
                declared_name: None,
            },
            PackageInfo {
                id: Some(2),
                name: "py".into(),
                path: "services/py".into(),
                kind: Some("python".into()),
                manifest: None,
                declared_name: None,
            },
        ];

        // Workspace-wide presence (union) carries both languages — what the
        // legacy code path saw.
        let workspace_langs = vec!["kotlin".to_string(), "python".to_string()];

        // Per-package map: each package only sees its own language.
        let mut per_pkg: HashMap<i64, HashSet<String>> = HashMap::new();
        per_pkg.insert(1, HashSet::from(["kotlin".to_string()]));
        per_pkg.insert(2, HashSet::from(["python".to_string()]));

        let registry = ecosystem::default_registry();
        let ctx = ProjectContext::initialize_with_per_package_languages(
            root,
            &packages,
            workspace_langs,
            per_pkg,
            registry,
        );

        let kotlin_stdlib = EcosystemId::new("kotlin-stdlib");
        let cpython_stdlib = EcosystemId::new("cpython-stdlib");

        assert!(
            ctx.active_ecosystems_by_package[&1].contains(&kotlin_stdlib),
            "Kotlin package must activate kotlin-stdlib"
        );
        assert!(
            !ctx.active_ecosystems_by_package[&1].contains(&cpython_stdlib),
            "Kotlin package must NOT activate cpython-stdlib (no .py files in pkg)"
        );

        assert!(
            ctx.active_ecosystems_by_package[&2].contains(&cpython_stdlib),
            "Python package must activate cpython-stdlib"
        );
        assert!(
            !ctx.active_ecosystems_by_package[&2].contains(&kotlin_stdlib),
            "Python package must NOT activate kotlin-stdlib (no .kt files in pkg)"
        );

        // Workspace-wide actives must still be the union (legacy consumers
        // see both stdlibs; per-package consumers narrow correctly).
        assert!(ctx.active_ecosystems.contains(&kotlin_stdlib));
        assert!(ctx.active_ecosystems.contains(&cpython_stdlib));
    }

    /// Phase 5: when the per-package language map is empty (caller passes
    /// `HashMap::new()`), the per-package evaluator falls back to
    /// workspace-wide language presence. Pre-Phase-5 behavior preserved.
    #[test]
    fn empty_per_package_language_map_falls_back_to_workspace_wide() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(root, "apps/a/dummy.kt", "fun main() {}\n");
        write_file(root, "apps/b/dummy.kt", "fun main() {}\n");

        let packages = vec![
            PackageInfo {
                id: Some(1),
                name: "a".into(),
                path: "apps/a".into(),
                kind: Some("kotlin".into()),
                manifest: None,
                declared_name: None,
            },
            PackageInfo {
                id: Some(2),
                name: "b".into(),
                path: "apps/b".into(),
                kind: Some("kotlin".into()),
                manifest: None,
                declared_name: None,
            },
        ];

        let registry = ecosystem::default_registry();
        let ctx = ProjectContext::initialize(
            root,
            &packages,
            vec!["kotlin".to_string()],
            registry,
        );

        let kotlin_stdlib = EcosystemId::new("kotlin-stdlib");
        // Both packages activate kotlin-stdlib because workspace-wide
        // language_presence contains "kotlin" and the per-package map
        // is empty — legacy fallback behavior.
        assert!(ctx.active_ecosystems_by_package[&1].contains(&kotlin_stdlib));
        assert!(ctx.active_ecosystems_by_package[&2].contains(&kotlin_stdlib));
    }
}
