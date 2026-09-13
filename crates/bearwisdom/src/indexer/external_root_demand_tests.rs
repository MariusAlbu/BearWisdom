use std::path::PathBuf;

use super::union_module_demand;
use crate::ecosystem::externals::ExternalDepRoot;

fn root(
    ecosystem: &'static str,
    module_path: &str,
    dir: &str,
    requested: &[&str],
) -> ExternalDepRoot {
    ExternalDepRoot {
        module_path: module_path.to_string(),
        version: String::from("unknown"),
        root: PathBuf::from(dir),
        ecosystem,
        package_id: None,
        requested_imports: requested.iter().map(|s| s.to_string()).collect(),
    }
}

/// The copy of a module that carries the built files is often discovered by a
/// package that imports nothing but the bare specifier. Demand belongs to the
/// module, so every copy must be probed for the whole workspace's subpaths.
#[test]
fn demand_reaches_every_copy_of_a_module() {
    let mut roots = vec![
        root("typescript", "pkg", "/a/node_modules/pkg", &["pkg/link", "pkg/legacy/image"]),
        root("typescript", "pkg", "/b/node_modules/pkg", &[]),
    ];
    union_module_demand(&mut roots);

    let expected = vec!["pkg/legacy/image".to_string(), "pkg/link".to_string()];
    assert_eq!(roots[0].requested_imports, expected);
    assert_eq!(roots[1].requested_imports, expected);
}

/// A module name that is a string prefix of another must not absorb its
/// demand — the match is anchored on a `/` segment boundary.
#[test]
fn unrelated_modules_do_not_bleed() {
    let mut roots = vec![
        root("typescript", "pkg-extra", "/a/node_modules/pkg-extra", &["pkg-extra/x"]),
        root("typescript", "pkg", "/a/node_modules/pkg", &[]),
    ];
    union_module_demand(&mut roots);

    assert_eq!(roots[0].requested_imports, vec!["pkg-extra/x".to_string()]);
    assert!(roots[1].requested_imports.is_empty());
}

/// Ecosystems whose demand is phrased as fully-qualified type names carry
/// specifiers that no module prefix matches; those stay with the root that
/// collected them so the union is a no-op outside path-shaped ecosystems.
#[test]
fn foreign_namespace_demand_is_not_propagated() {
    let mut roots = vec![
        root(
            "java",
            "com.acme:lib",
            "/cache/com/acme/lib/1.0",
            &["org.spring.context.Ctx"],
        ),
        root("java", "com.acme:lib", "/other/com/acme/lib/1.0", &[]),
    ];
    union_module_demand(&mut roots);

    assert_eq!(
        roots[0].requested_imports,
        vec!["org.spring.context.Ctx".to_string()]
    );
    assert!(roots[1].requested_imports.is_empty());
}

/// The widened set drives first-writer-wins selection downstream, so it must
/// be sorted and a second application must change nothing.
#[test]
fn union_is_sorted_and_idempotent() {
    let mut roots = vec![
        root("typescript", "pkg", "/a/node_modules/pkg", &["pkg/z", "pkg/a"]),
        root("typescript", "pkg", "/b/node_modules/pkg", &["pkg/m"]),
    ];
    union_module_demand(&mut roots);
    let once: Vec<Vec<String>> = roots.iter().map(|r| r.requested_imports.clone()).collect();
    assert_eq!(
        once[0],
        vec!["pkg/a".to_string(), "pkg/m".to_string(), "pkg/z".to_string()]
    );

    union_module_demand(&mut roots);
    let twice: Vec<Vec<String>> = roots.iter().map(|r| r.requested_imports.clone()).collect();
    assert_eq!(once, twice);
}

/// Two ecosystems may name the same module path; their demand sets are
/// separate universes and must not merge.
#[test]
fn ecosystem_scopes_the_union() {
    let mut roots = vec![
        root("typescript", "pkg", "/a/node_modules/pkg", &["pkg/link"]),
        root("go", "pkg", "/gopath/pkg", &[]),
    ];
    union_module_demand(&mut roots);

    assert_eq!(roots[0].requested_imports, vec!["pkg/link".to_string()]);
    assert!(roots[1].requested_imports.is_empty());
}
