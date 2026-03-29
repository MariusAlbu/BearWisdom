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
    let mut ctx = ProjectContext::default();
    ctx.external_prefixes.insert("System".to_string());
    ctx.external_prefixes.insert("Newtonsoft.Json".to_string());
    ctx.external_prefixes.insert("MediatR".to_string());

    assert!(ctx.is_external_namespace("System"));
    assert!(ctx.is_external_namespace("System.Linq"));
    assert!(ctx.is_external_namespace("System.Collections.Generic"));
    assert!(ctx.is_external_namespace("Newtonsoft.Json"));
    assert!(ctx.is_external_namespace("Newtonsoft.Json.Linq"));
    assert!(ctx.is_external_namespace("MediatR"));
    assert!(!ctx.is_external_namespace("App.Models"));
    assert!(!ctx.is_external_namespace("eShop.Catalog"));
    // "Systemx" should not match prefix "System"
    assert!(!ctx.is_external_namespace("Systemx"));
}

#[test]
fn test_most_capable_sdk() {
    assert_eq!(most_capable_sdk(&[DotnetSdkType::Base, DotnetSdkType::Web]), DotnetSdkType::Web);
    assert_eq!(most_capable_sdk(&[DotnetSdkType::Worker]), DotnetSdkType::Worker);
    assert_eq!(most_capable_sdk(&[]), DotnetSdkType::Other);
    assert_eq!(most_capable_sdk(&[DotnetSdkType::Base]), DotnetSdkType::Base);
}
