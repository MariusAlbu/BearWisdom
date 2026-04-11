//! Integration test for .NET externals via NuGet global packages cache.
//!
//! Shells out to the installed `dotnet` SDK to build a tiny class library,
//! then stages it under a synthetic `~/.nuget/packages/` layout so the
//! externals discovery code can find it via `BEARWISDOM_NUGET_PACKAGES`.
//! Points a tiny consumer project at it, indexes, and asserts that the
//! DLL's types landed in the index as external symbols.
//!
//! Skipped automatically when `dotnet` isn't on PATH — CI environments
//! without the .NET SDK still pass the rest of the suite.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// Return the full path to `dotnet` if it's on PATH, else `None`.
/// Uses `where` on Windows / `which` on Unix via `dotnet --version` to
/// avoid needing a which crate.
fn dotnet_available() -> bool {
    Command::new("dotnet")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a tiny class library with one public class and two public
/// methods, then return the path to the produced DLL.
fn build_fake_library() -> Option<(TempDir, PathBuf)> {
    let work = TempDir::new().unwrap();
    let proj_dir = work.path().join("FakeLib");
    fs::create_dir_all(&proj_dir).unwrap();

    // Minimal SDK-style csproj.
    fs::write(
        proj_dir.join("FakeLib.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <AssemblyName>FakeLib</AssemblyName>
    <RootNamespace>FakeExt</RootNamespace>
    <Nullable>enable</Nullable>
  </PropertyGroup>
</Project>
"#,
    )
    .unwrap();

    fs::write(
        proj_dir.join("Greeter.cs"),
        r#"namespace FakeExt;

public class Greeter
{
    private readonly string _prefix;

    public Greeter(string prefix)
    {
        _prefix = prefix;
    }

    public string Greet(string name)
    {
        return _prefix + " " + name;
    }

    public int Count(string text)
    {
        return text.Length;
    }
}

public interface IFormatter
{
    string Format(string input);
}
"#,
    )
    .unwrap();

    let output = Command::new("dotnet")
        .arg("build")
        .arg("-c")
        .arg("Release")
        .arg("--nologo")
        .current_dir(&proj_dir)
        .output()
        .ok()?;

    if !output.status.success() {
        eprintln!(
            "dotnet build failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }

    let dll = proj_dir
        .join("bin")
        .join("Release")
        .join("net8.0")
        .join("FakeLib.dll");
    if !dll.is_file() {
        return None;
    }
    Some((work, dll))
}

/// Stage the built DLL under a synthetic NuGet cache layout.
fn seed_fake_nuget_cache(dll_path: &std::path::Path) -> TempDir {
    let cache = TempDir::new().unwrap();
    let pkg_dir = cache
        .path()
        .join("fakelib")
        .join("1.0.0")
        .join("lib")
        .join("net8.0");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::copy(dll_path, pkg_dir.join("FakeLib.dll")).unwrap();
    cache
}

fn seed_consumer_project() -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file(
        "Consumer.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="FakeLib" Version="1.0.0" />
  </ItemGroup>
</Project>
"#,
    );

    project.add_file(
        "Program.cs",
        r#"using FakeExt;

namespace Consumer;

public class Program
{
    public static void Main(string[] args)
    {
        var g = new Greeter("Hello,");
        System.Console.WriteLine(g.Greet("world"));
    }
}
"#,
    );

    project
}

#[test]
fn external_dotnet_package_is_indexed_and_resolved() {
    if !dotnet_available() {
        eprintln!("dotnet SDK not available, skipping .NET externals integration test");
        return;
    }
    let Some((_lib_work, dll_path)) = build_fake_library() else {
        eprintln!("failed to build fake library, skipping");
        return;
    };

    let cache = seed_fake_nuget_cache(&dll_path);
    let project = seed_consumer_project();

    let prior_cache = std::env::var_os("BEARWISDOM_NUGET_PACKAGES");
    // SAFETY: std::env::set_var is process-global. Test owns it for the
    // duration and restores it afterward so sibling tests see the original
    // environment.
    unsafe {
        std::env::set_var("BEARWISDOM_NUGET_PACKAGES", cache.path());
    }

    let mut db = TestProject::in_memory_db();
    let stats = full_index(&mut db, project.path(), None, None, None).unwrap();

    unsafe {
        match prior_cache {
            Some(v) => std::env::set_var("BEARWISDOM_NUGET_PACKAGES", v),
            None => std::env::remove_var("BEARWISDOM_NUGET_PACKAGES"),
        }
    }

    // Internal symbols exist (Program, Main).
    assert!(
        stats.symbol_count >= 1,
        "expected at least one internal symbol, got {}",
        stats.symbol_count
    );

    // External symbols from FakeLib.dll: Greeter + IFormatter + their
    // public methods. Also includes Object-inherited members? No — we only
    // scan types defined in the assembly, not base types.
    let external_files: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM files WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_files >= 1,
        "expected at least 1 external file (synthetic FakeLib entry), got {external_files}"
    );

    let external_symbols: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols WHERE origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_symbols >= 4,
        "expected at least Greeter + IFormatter + 2 methods, got {external_symbols}"
    );

    // The Greeter type must be in the index under its full name.
    let greeter_exists: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'FakeExt.Greeter' AND origin = 'external'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(greeter_exists, 1, "FakeExt.Greeter missing from external index");

    // IFormatter interface must land too.
    let iformatter_exists: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'FakeExt.IFormatter'
               AND origin = 'external' AND kind = 'interface'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        iformatter_exists, 1,
        "FakeExt.IFormatter interface missing from external index"
    );

    // Method Greet should land with qualified_name = FakeExt.Greeter.Greet.
    let greet_method: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols
             WHERE qualified_name = 'FakeExt.Greeter.Greet'
               AND origin = 'external' AND kind = 'method'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        greet_method, 1,
        "FakeExt.Greeter.Greet method missing from external index"
    );
}
