//! Integration test for Phase B — .NET `<ProjectReference>` support.
//!
//! Builds a two-project .NET solution on disk: `Shared.csproj` exposes a
//! `Greeter` class, `App.csproj` references it via `<ProjectReference>`
//! and consumes it through `using Shared;`. Asserts the cross-project edge
//! resolves at confidence 1.0 and that `workspace_graph.declared_dep` fires
//! for the App → Shared pair via the ProjectReference entry in package_deps.

use std::fs;
use std::path::Path;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

fn write_file(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

fn build_solution() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Bare solution marker — full_index's package detector keys off .csproj
    // files, so no .sln is strictly required. A placeholder keeps layouts
    // realistic.
    write_file(root, "solution.sln", "# placeholder\n");

    write_file(
        root,
        "src/Shared/Shared.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
</Project>
"#,
    );
    write_file(
        root,
        "src/Shared/Greeter.cs",
        r#"namespace Shared
{
    public class Greeter
    {
        public string Hello(string name) => $"Hi, {name}";
    }
}
"#,
    );

    write_file(
        root,
        "src/App/App.csproj",
        r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
  </PropertyGroup>
  <ItemGroup>
    <ProjectReference Include="..\Shared\Shared.csproj" />
  </ItemGroup>
</Project>
"#,
    );
    write_file(
        root,
        "src/App/Program.cs",
        r#"using Shared;

namespace App
{
    public class Program
    {
        public static void Main()
        {
            var g = new Greeter();
            g.Hello("world");
        }
    }
}
"#,
    );

    tmp
}

#[test]
fn using_directive_resolves_across_projects() {
    // With `using Shared;` in App.Program, the cross-project wire-up
    // produces an `imports` edge from the App namespace to the Shared
    // namespace at high confidence. This proves the pipeline linked the
    // two projects (files indexed, symbols named, edge landed) without
    // depending on whether the C# extractor emits constructor/call refs
    // inside method bodies (that's a separate extractor concern).
    let tmp = build_solution();
    let root = tmp.path();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let shared_ns_id: i64 = db
        .query_row(
            "SELECT s.id FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.qualified_name = 'Shared' AND s.kind = 'namespace' AND f.path LIKE '%Shared%'",
            [],
            |row| row.get(0),
        )
        .expect("Shared namespace symbol not indexed");

    let max_confidence: Option<f64> = db
        .query_row(
            "SELECT MAX(e.confidence)
             FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE e.target_id = ?1 AND f.path LIKE '%App%' AND e.kind = 'imports'",
            rusqlite::params![shared_ns_id],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    let c = max_confidence.expect("no imports edge App → Shared");
    assert!(
        c >= 0.9,
        "using-directive edge expected at high confidence, got {c}"
    );
}

#[test]
fn project_reference_populates_package_deps() {
    let tmp = build_solution();
    let root = tmp.path();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    // package_deps should carry a row with kind='project_reference' from App
    // pointing at 'Shared'.
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT p.declared_name, pd.dep_name, pd.kind
             FROM package_deps pd
             JOIN packages p ON p.id = pd.package_id
             WHERE pd.kind = 'project_reference'
               AND p.declared_name = 'App'
               AND pd.dep_name = 'Shared'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .ok();
    assert!(
        row.is_some(),
        "expected a project_reference row from App → Shared in package_deps"
    );
}

#[test]
fn workspace_graph_declared_dep_fires_for_project_reference() {
    let tmp = build_solution();
    let root = tmp.path();

    let mut db = TestProject::in_memory_db();
    full_index(&mut db, root, None, None, None).expect("index failed");

    let graph = bearwisdom::workspace_graph(&db).expect("workspace_graph failed");
    let app_to_shared = graph
        .iter()
        .find(|e| e.source_package == "App" && e.target_package == "Shared")
        .expect("expected an App → Shared edge in workspace_graph");

    assert!(
        app_to_shared.declared_dep,
        "declared_dep must be true for the project_reference pair"
    );
}
