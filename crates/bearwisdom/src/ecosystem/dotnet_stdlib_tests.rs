// =============================================================================
// dotnet_stdlib_tests.rs — .NET shared-framework ecosystem surface
// =============================================================================

use super::*;
use std::fs;
use tempfile::TempDir;

fn dep_root(root: &Path) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: "Microsoft.NETCore.App".to_string(),
        version: String::new(),
        root: root.to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    }
}

#[test]
fn version_pick_compares_major_numerically() {
    let dirs = vec![
        PathBuf::from("/dotnet/shared/Microsoft.NETCore.App/8.0.7"),
        PathBuf::from("/dotnet/shared/Microsoft.NETCore.App/10.0.8"),
    ];
    let picked = pick_framework_version(&dirs).expect("a version must be picked");
    assert_eq!(
        picked.file_name().unwrap().to_str().unwrap(),
        "10.0.8",
        "10 outranks 8 numerically; a lexicographic sort picks 8.0.7"
    );
}

#[test]
fn version_pick_compares_patch_numerically() {
    let dirs = vec![
        PathBuf::from("/dotnet/shared/Microsoft.NETCore.App/6.0.32"),
        PathBuf::from("/dotnet/shared/Microsoft.NETCore.App/6.0.16"),
    ];
    let picked = pick_framework_version(&dirs).expect("a version must be picked");
    assert_eq!(picked.file_name().unwrap().to_str().unwrap(), "6.0.32");
}

#[test]
fn version_pick_prefers_release_over_preview() {
    let dirs = vec![
        PathBuf::from("/dotnet/shared/Microsoft.NETCore.App/9.0.0-rc.2"),
        PathBuf::from("/dotnet/shared/Microsoft.NETCore.App/9.0.0"),
    ];
    let picked = pick_framework_version(&dirs).expect("a version must be picked");
    assert_eq!(picked.file_name().unwrap().to_str().unwrap(), "9.0.0");
}

#[test]
fn version_pick_none_without_installs() {
    assert!(pick_framework_version(&[]).is_none());
}

#[test]
fn locator_parse_metadata_only_returns_none() {
    // The locator carries no override, so the trait default (None) answers —
    // no DLL is cracked at index time.
    let tmp = TempDir::new().unwrap();
    let result = ExternalSourceLocator::parse_metadata_only(&DotnetStdlibEcosystem, tmp.path());
    assert!(result.is_none(), "locator must not offer an eager dump");
}

#[test]
fn ecosystem_parse_metadata_only_returns_none() {
    let tmp = TempDir::new().unwrap();
    let dep = dep_root(tmp.path());
    let eco = DotnetStdlibEcosystem;
    let result = <DotnetStdlibEcosystem as Ecosystem>::parse_metadata_only(&eco, &dep);
    assert!(result.is_none(), "ecosystem must not offer an eager dump");
}

#[test]
fn demand_driven_flag_is_set() {
    assert!(DotnetStdlibEcosystem.uses_demand_driven_parse());
}

#[test]
fn activation_stays_language_present() {
    // The BCL is a substrate: every CLR-family source file reaches it without
    // declaring it in a manifest.
    match DotnetStdlibEcosystem.activation() {
        EcosystemActivation::Any(clauses) => {
            assert!(clauses
                .iter()
                .all(|c| matches!(c, EcosystemActivation::LanguagePresent(_))));
        }
        other => panic!("expected LanguagePresent clauses, got {other:?}"),
    }
}

#[test]
fn symbol_index_tolerates_unreadable_dlls() {
    // A file that ends in .dll but carries no ECMA-335 metadata is declined by
    // the type enumerator — the offering stays empty instead of panicking.
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("System.Bogus.dll"), b"not a PE file").unwrap();
    fs::write(tmp.path().join("notes.txt"), b"ignored").unwrap();
    let index = DotnetStdlibEcosystem.build_symbol_index(&[dep_root(tmp.path())]);
    assert!(index.is_empty());
}

#[test]
fn symbol_index_empty_for_missing_root() {
    let index =
        DotnetStdlibEcosystem.build_symbol_index(&[dep_root(Path::new("/no/such/framework"))]);
    assert!(index.is_empty());
}

/// Offering format over a real framework assembly. Skipped when the machine has
/// no .NET install — the assertion needs genuine ECMA-335 metadata, and one
/// copied DLL keeps the scan to a single parse.
#[test]
fn symbol_index_offers_dotnet_type_paths_for_real_assembly() {
    let Some(framework) = probe_shared_framework_dir() else {
        return;
    };
    let source_dll = framework.join("System.Console.dll");
    if !source_dll.is_file() {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let staged = tmp.path().join("System.Console.dll");
    fs::copy(&source_dll, &staged).unwrap();

    let index = DotnetStdlibEcosystem.build_symbol_index(&[dep_root(tmp.path())]);
    assert!(
        !index.is_empty(),
        "public types of a framework assembly must be offered"
    );
    let console = index
        .locate("Microsoft.NETCore.App", "Console")
        .expect("System.Console must be offered under the framework module");
    let encoded = console.to_string_lossy();
    assert!(
        encoded.starts_with("ext:dotnet-type:"),
        "offering must use NuGet's virtual-path scheme so the shared \
         materialize path cracks it: {encoded}"
    );
    assert_eq!(
        encoded.matches("!!").count(),
        2,
        "path must encode <dll>!!<assembly>!!<QualifiedType>: {encoded}"
    );
    assert!(encoded.ends_with("!!System.Console"), "{encoded}");
}
