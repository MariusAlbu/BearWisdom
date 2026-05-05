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

