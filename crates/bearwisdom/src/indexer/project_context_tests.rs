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
            },
            PackageInfo {
                id: Some(2),
                name: "web".into(),
                path: "web".into(),
                kind: Some("npm".into()),
                manifest: Some("web/package.json".into()),
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
            },
            PackageInfo {
                id: Some(2),
                name: "empty".into(),
                path: "packages/empty".into(),
                kind: None,
                manifest: None,
            },
        ];

        let ctx = build_project_context_with_packages(root, &packages);
        assert!(ctx.by_package.contains_key(&1));
        assert!(!ctx.by_package.contains_key(&2));
        // Package 2 (empty) falls back to union — sees express.
        assert!(ctx.has_dependency_for(Some(2), ManifestKind::Npm, "express"));
    }
}

