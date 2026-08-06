// =============================================================================
// nuget/manifest_tests.rs — csproj manifest parsing tests
// =============================================================================

use super::*;

#[test]
fn using_items_collect_include_namespaces() {
    let csproj = r#"
        <Project Sdk="Microsoft.NET.Sdk">
          <ItemGroup>
            <Using Include="Xunit" />
            <Using Include="FakeItEasy" />
            <Using Include="FluentAssertions" />
          </ItemGroup>
        </Project>
    "#;
    assert_eq!(
        parse_using_items(csproj),
        vec!["Xunit", "FakeItEasy", "FluentAssertions"]
    );
}

#[test]
fn using_items_skip_remove_alias_and_static() {
    let csproj = r#"
        <ItemGroup>
          <Using Remove="System.Net.Http" />
          <Using Include="MyAlias.Target" Alias="MyAlias" />
          <Using Include="System.Math" Static="true" />
          <Using Include="NodaTime" />
        </ItemGroup>
    "#;
    assert_eq!(parse_using_items(csproj), vec!["NodaTime"]);
}

#[test]
fn using_items_dedup_and_ignore_other_tags() {
    let csproj = r#"
        <ItemGroup>
          <Using Include="Xunit" />
          <Using Include="Xunit" />
          <PackageReference Include="xunit.v3" Version="3.2.2" />
        </ItemGroup>
    "#;
    assert_eq!(parse_using_items(csproj), vec!["Xunit"]);
}
