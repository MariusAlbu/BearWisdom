// =============================================================================
// nuget/mod_tests.rs — unit tests for the NuGet ecosystem surface
// =============================================================================

use super::cs_header::scan_cs_header;
use super::signature_format::{
    format_generic_suffix, strip_backtick_arity, substitute_generic_placeholders,
};
use super::source_discovery::discover_nuget_source_files;
use super::*;

#[test]
fn ecosystem_identity() {
    let n = NugetEcosystem;
    assert_eq!(n.id(), ID);
    assert_eq!(Ecosystem::kind(&n), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&n), &["csharp", "fsharp", "vbnet"]);
}

#[test]
fn legacy_locator_tag_is_dotnet() {
    assert_eq!(ExternalSourceLocator::ecosystem(&NugetEcosystem), "dotnet");
}

#[test]
fn demand_driven_flag_is_set() {
    assert!(NugetEcosystem.uses_demand_driven_parse());
}

#[test]
fn parse_metadata_only_returns_none() {
    // The eager dump must be disabled — demand-driven path handles all DLL
    // extraction. An empty project dir must not trigger the old eager pass.
    let tmp = std::env::temp_dir().join("bw-nuget-test-empty-project");
    std::fs::create_dir_all(&tmp).unwrap();
    let result = ExternalSourceLocator::parse_metadata_only(&NugetEcosystem, &tmp);
    assert!(result.is_none(), "parse_metadata_only must return None");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn strip_backtick_arity_removes_generic_suffix() {
    assert_eq!(strip_backtick_arity("Repository`1"), "Repository");
    assert_eq!(strip_backtick_arity("Dictionary`2"), "Dictionary");
    assert_eq!(strip_backtick_arity("Func`4"), "Func");
    assert_eq!(strip_backtick_arity("List"), "List");
}

#[test]
fn format_generic_suffix_joins_names() {
    assert_eq!(format_generic_suffix(&[]), "");
    assert_eq!(format_generic_suffix(&["T".to_string()]), "<T>");
    assert_eq!(
        format_generic_suffix(&["T".to_string(), "U".to_string()]),
        "<T, U>"
    );
}

#[test]
fn substitute_placeholders_swaps_ecma335_syntax() {
    let type_gen = vec!["T".to_string()];
    let method_gen = vec!["U".to_string(), "V".to_string()];
    assert_eq!(
        substitute_generic_placeholders("!!0", &type_gen, &method_gen),
        "U"
    );
    assert_eq!(
        substitute_generic_placeholders("!!1", &type_gen, &method_gen),
        "V"
    );
    assert_eq!(
        substitute_generic_placeholders("!0", &type_gen, &method_gen),
        "T"
    );
    assert_eq!(
        substitute_generic_placeholders("Func<!0, !!0, !!1>", &type_gen, &method_gen),
        "Func<T, U, V>"
    );
    assert_eq!(
        substitute_generic_placeholders("!!5", &type_gen, &method_gen),
        "!!5"
    );
}

#[test]
fn substitute_placeholders_multi_digit_indices() {
    let method_gen: Vec<String> = (0..15).map(|i| format!("T{i}")).collect();
    assert_eq!(
        substitute_generic_placeholders("!!10", &[], &method_gen),
        "T10"
    );
    assert_eq!(
        substitute_generic_placeholders("!!14", &[], &method_gen),
        "T14"
    );
}

#[test]
fn project_references_extract_filename_stems() {
    let csproj = r#"
        <Project Sdk="Microsoft.NET.Sdk">
          <ItemGroup>
            <ProjectReference Include="../Shared/Shared.csproj" />
            <ProjectReference Include="..\Infra\Infra.fsproj" />
            <ProjectReference Include="./Legacy.vbproj" />
          </ItemGroup>
        </Project>
    "#;
    let refs = parse_project_references(csproj);
    assert_eq!(refs, vec!["Shared", "Infra", "Legacy"]);
}

#[test]
fn project_references_kept_separate_from_packages() {
    let csproj = r#"
        <Project Sdk="Microsoft.NET.Sdk">
          <ItemGroup>
            <PackageReference Include="Newtonsoft.Json" Version="13.0.1" />
            <ProjectReference Include="../Shared/Shared.csproj" />
          </ItemGroup>
        </Project>
    "#;
    let pkgs = parse_package_references(csproj);
    let prs = parse_project_references(csproj);
    assert_eq!(pkgs, vec!["Newtonsoft.Json"]);
    assert_eq!(prs, vec!["Shared"]);
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

// ---------------------------------------------------------------------------
// C# header scanner tests
// ---------------------------------------------------------------------------

#[test]
fn scan_cs_header_finds_class_and_interface() {
    let src = r#"
namespace MyLib.Core {
    public interface IRepository {
        IEnumerable<T> GetAll();
    }
    public class UserRepository : IRepository {
        public IEnumerable<T> GetAll() { return null; }
    }
}
"#;
    let decls = scan_cs_header(src);
    let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"IRepository"), "should find IRepository");
    assert!(
        names.contains(&"UserRepository"),
        "should find UserRepository"
    );
}

#[test]
fn scan_cs_header_captures_namespace_scope() {
    let src = r#"
namespace Acme.Orders {
    public class OrderService { }
}
"#;
    let decls = scan_cs_header(src);
    let svc = decls
        .iter()
        .find(|d| d.name == "OrderService")
        .expect("OrderService missing");
    assert_eq!(svc.scope, "Acme.Orders");
}

#[test]
fn scan_cs_header_skips_private_members() {
    let src = r#"
namespace X {
    public class Foo {
        private void Secret() { }
        public void Public() { }
    }
}
"#;
    let decls = scan_cs_header(src);
    let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
    assert!(!names.contains(&"Secret"), "should not emit private method");
    assert!(names.contains(&"Public"), "should emit public method");
}

#[test]
fn scan_cs_header_handles_enum_and_struct() {
    let src = r#"
namespace Lib {
    public enum Status { Active, Inactive }
    public struct Point { public int X; public int Y; }
}
"#;
    let decls = scan_cs_header(src);
    let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"Status"));
    assert!(names.contains(&"Point"));
}

#[test]
fn discover_nuget_source_files_empty_for_missing_dir() {
    let tmp = std::env::temp_dir().join("bw-nuget-test-nonexistent-xyz");
    let files = discover_nuget_source_files(&tmp);
    assert!(files.is_empty());
}

#[test]
fn discover_nuget_source_finds_contentfiles() {
    let tmp = std::env::temp_dir().join("bw-nuget-test-contentfiles");
    let cs_dir = tmp.join("contentFiles").join("cs").join("any");
    std::fs::create_dir_all(&cs_dir).unwrap();
    std::fs::write(cs_dir.join("Helper.cs"), "public class Helper {}").unwrap();
    let files = discover_nuget_source_files(&tmp);
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("Helper.cs"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn discover_nuget_source_finds_src_dir() {
    let tmp = std::env::temp_dir().join("bw-nuget-test-src");
    let src_dir = tmp.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("MyClass.cs"), "public class MyClass {}").unwrap();
    let files = discover_nuget_source_files(&tmp);
    assert_eq!(files.len(), 1);
    let _ = std::fs::remove_dir_all(&tmp);
}
