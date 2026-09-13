//! Integration test for .NET constructor supply consumed from F#.
//!
//! F# construction syntax carries no `new` keyword, so `Greeter("Hello,")`
//! lands as an `EdgeKind::Calls` ref against a bare name. A supplied DLL type
//! whose emitted surface holds only Method/Property/Field rows has nothing
//! kind-compatible for that ref to bind, so the constructor rows the
//! ECMA-335 cracker emits are what closes it.
//!
//! Builds a tiny class library with the installed `dotnet` SDK, stages it
//! under a synthetic NuGet cache via `BEARWISDOM_NUGET_PACKAGES`, points an
//! F# consumer at it, indexes, and asserts the constructor row exists, binds,
//! and types the chain root of a member call on the constructed value.
//!
//! Skipped automatically when `dotnet` isn't on PATH.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn dotnet_available() -> bool {
    Command::new("dotnet")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build a tiny class library whose one public class has a public instance
/// constructor taking a string, then return the path to the produced DLL.
fn build_fake_library() -> Option<(TempDir, PathBuf)> {
    let work = TempDir::new().unwrap();
    let proj_dir = work.path().join("FakeLib");
    fs::create_dir_all(&proj_dir).unwrap();

    fs::write(
        proj_dir.join("FakeLib.csproj"),
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <AssemblyName>FakeLib</AssemblyName>
    <RootNamespace>FakeExt</RootNamespace>
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
        "Consumer.fsproj",
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
  <ItemGroup>
    <Compile Include="Program.fs" />
  </ItemGroup>
  <ItemGroup>
    <PackageReference Include="FakeLib" Version="1.0.0" />
  </ItemGroup>
</Project>
"#,
    );

    project.add_file(
        "Program.fs",
        r#"module Consumer.Program

open FakeExt

let make () = Greeter("Hello,")

let greet () = Greeter("Hello,").Greet "world"
"#,
    );

    project
}

#[test]
fn fsharp_construction_binds_the_supplied_constructor_row() {
    if !dotnet_available() {
        eprintln!("dotnet SDK not available, skipping .NET constructor integration test");
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
    let indexed = full_index(&mut db, project.path(), None, None, None);

    unsafe {
        match prior_cache {
            Some(v) => std::env::set_var("BEARWISDOM_NUGET_PACKAGES", v),
            None => std::env::remove_var("BEARWISDOM_NUGET_PACKAGES"),
        }
    }
    indexed.unwrap();

    // (a) Supply precondition: the cracker emits Constructor rows at all.
    let external_ctors: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE f.path LIKE 'ext:dotnet-type:%' AND s.kind = 'constructor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        external_ctors > 0,
        "no constructor rows in the DLL supply surface"
    );

    // The constructor is named after its type and carries the constructed type
    // in its return slot — the slot a callable row's yield is decoded from.
    let ctor_signature: String = db
        .query_row(
            "SELECT signature FROM symbols
             WHERE qualified_name = 'FakeExt.Greeter.Greeter' AND kind = 'constructor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ctor_signature, "Greeter(string): FakeExt.Greeter");

    // The ordinary-method rendering is unchanged by the shared parameter-list
    // extraction.
    let greet_signature: String = db
        .query_row(
            "SELECT signature FROM symbols
             WHERE qualified_name = 'FakeExt.Greeter.Greet' AND kind = 'method'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(greet_signature, "Greet(string): string");

    // (b) The bare application binds to the constructor row through the
    // unmodified ladder.
    let ctor_edges: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols t ON t.id = e.target_id
             JOIN symbols s ON s.id = e.source_id
             WHERE e.kind = 'calls' AND s.name = 'make'
               AND t.kind = 'constructor'
               AND t.qualified_name = 'FakeExt.Greeter.Greeter'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ctor_edges, 1,
        "make -> FakeExt.Greeter.Greeter calls edge missing"
    );

    // (c) The ref dies as resolved, not merely as one more candidate.
    let unresolved_greeter: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM unresolved_refs
             WHERE target_name = 'Greeter' AND drained = 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        unresolved_greeter, 0,
        "Greeter still unresolved after the constructor row landed"
    );

    // (d) The constructed value types the chain root, so the member call on it
    // resolves.
    let member_edges: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols t ON t.id = e.target_id
             JOIN symbols s ON s.id = e.source_id
             WHERE e.kind = 'calls' AND s.name = 'greet'
               AND t.qualified_name = 'FakeExt.Greeter.Greet'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        member_edges, 1,
        "Greet on the constructed value did not resolve"
    );
}
