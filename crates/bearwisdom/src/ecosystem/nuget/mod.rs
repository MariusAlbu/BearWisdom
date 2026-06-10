// =============================================================================
// ecosystem/nuget/ — NuGet ecosystem (.NET: C#, F#, VB.NET)
//
// Phase 2 + 3: consolidates `indexer/externals/dotnet.rs` +
// `indexer/manifest/nuget.rs`. .NET externals are metadata-only: DLLs are
// parsed via the `dotscope` ECMA-335 reader and emitted as synthetic
// `ParsedFile` rows — no source walk. The pipeline uses
// `parse_metadata_only()` instead of the usual locate_roots/walk_root path.
//
// Languages: csharp, fsharp, vbnet. All three consume the same DLLs from
// `~/.nuget/packages/`. The file-level `language` tag on emitted parsed
// files follows the owning .csproj/.fsproj/.vbproj file type.
//
// Hybrid source + metadata strategy (additive, no flag flip):
//   The DLL metadata path (`parse_metadata_only`) remains the primary eager
//   pass — `uses_demand_driven_parse` stays `false`. A supplementary source
//   scan runs alongside it: for each resolved package directory we look for
//   `.cs` files under `contentFiles/cs/<tfm>/`, `lib/<tfm>/`, `src/`, and
//   the package root. Any found source files are parsed header-only
//   (top-level namespace/class/interface/enum/struct/method decls) and emitted
//   as additional `ParsedFile` rows. When source and DLL metadata provide the
//   same qname, source wins at query time because it carries real line numbers.
// =============================================================================

use std::path::Path;
use std::sync::Arc;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::walker::WalkedFile;

mod cs_header;
mod dll_metadata;
mod manifest;
mod source_discovery;
mod symbol_index;

pub(crate) use dll_metadata::parse_dotnet_dll_public;
pub use dll_metadata::{nuget_packages_root, parse_dotnet_externals};
pub use manifest::{
    implicit_usings_for_sdk, most_capable_sdk, parse_global_usings, parse_package_references,
    parse_package_references_full, parse_project_references, parse_sdk_type, DotnetSdkType,
    NuGetCoord, NuGetManifest,
};

use dll_metadata::parse_dotnet_externals_with_source;
use symbol_index::{build_nuget_source_symbol_index, resolve_nuget_source_symbols};

pub const ID: EcosystemId = EcosystemId::new("nuget");

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &["csharp", "fsharp", "vbnet"];
const LEGACY_ECOSYSTEM_TAG: &str = "dotnet";

pub struct NugetEcosystem;

impl Ecosystem for NugetEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }
    fn manifest_specs(&self) -> &'static [ManifestSpec] {
        MANIFESTS
    }

    fn workspace_package_extensions(&self) -> &'static [(&'static str, &'static str)] {
        // .NET project files embed the project name as the filename stem,
        // so they must be matched by extension. One row per project file.
        &[
            (".csproj", "dotnet"),
            (".fsproj", "dotnet"),
            (".vbproj", "dotnet"),
        ]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        // No `packages/` here — that's a canonical npm-monorepo workspace
        // directory and pruning it would block sibling Dart/iOS/Rust pkgs.
        // NuGet's package cache lives at `~/.nuget/packages/`, not in the
        // repo.
        &["bin", "obj", ".nuget"]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `*.csproj` / `*.fsproj` / `*.vbproj` (or
        // `Directory.Packages.props`). The .NET runtime + BCL belong to
        // `dotnet-stdlib`; nuget only resolves declared NuGet package
        // refs. Dropping the LanguagePresent shotgun is correct per the
        // trait doc.
        EcosystemActivation::ManifestMatch
    }

    // NuGet is metadata-only: no source dep roots, no walking. Return empty
    // from locate_roots so the pipeline knows there's nothing to walk; the
    // legacy indexer consumes parse_metadata_only() directly below.
    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        Vec::new()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    // `uses_demand_driven_parse` intentionally stays `false`.
    //
    // The DLL metadata path is the primary eager pass — flipping this to
    // `true` would disable `parse_metadata_only` and leave the indexer
    // relying only on the demand loop, which requires `locate_roots` to return
    // real dep roots. Since `locate_roots` returns empty (NuGet has no source
    // walk), flipping would cause a complete regression.
    //
    // The new source-index path is SUPPLEMENTARY: it runs inside
    // `parse_metadata_only` alongside the DLL scan, so source wins on
    // qnames it covers while DLL metadata fills the rest. No flag change
    // needed.

    /// Build a supplementary `(module, name) → file` index over any `.cs`
    /// source files found inside NuGet package dirs. Consumed by chain walkers
    /// that need a file path for a specific qname — when source resolves it,
    /// source wins over the DLL-synthesized row.
    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_nuget_source_symbol_index(dep_roots)
    }

    /// Resolve a specific import against the supplementary source index.
    /// Falls back to empty when no source covers the requested symbols.
    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        _package: &str,
        symbols: &[&str],
    ) -> Vec<WalkedFile> {
        resolve_nuget_source_symbols(dep, symbols)
    }

    /// Resolve a single fully-qualified name from the source index.
    /// Falls back to empty when no source covers `fqn`.
    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
        let short = fqn.rsplit('.').next().unwrap_or(fqn);
        resolve_nuget_source_symbols(dep, &[short])
    }
}

impl ExternalSourceLocator for NugetEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<crate::types::ParsedFile>> {
        let (mut parsed, source_pf) = parse_dotnet_externals_with_source(project_root);
        parsed.extend(source_pf);
        if parsed.is_empty() {
            None
        } else {
            Some(parsed)
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<NugetEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NugetEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::cs_header::scan_cs_header;
    use super::dll_metadata::{
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

    // -----------------------------------------------------------------------
    // C# header scanner tests
    // -----------------------------------------------------------------------

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
}
