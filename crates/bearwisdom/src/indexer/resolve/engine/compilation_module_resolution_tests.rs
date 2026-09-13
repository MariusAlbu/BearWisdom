// =============================================================================
// compilation_module_resolution_tests — the declared-source-root module link
// =============================================================================

use std::sync::Arc;

use super::*;
use crate::indexer::resolve::engine::module_graph::ModuleGraph;
use crate::indexer::resolve::engine::module_input::ModuleInput;
use crate::indexer::resolve::engine::module_paths::PathRules;
use crate::indexer::resolve::engine::testkit::Lookup;
use crate::type_checker::core::types::TypeArena;

const INDEXED: &[&str] = &[
    "packages/core_client/lib/core_client.dart",
    "packages/core_client/lib/src/codec.dart",
    "packages/app_server/lib/endpoint.dart",
];

fn source_rules() -> PathRules {
    PathRules {
        extensions: vec![".dart".into()],
        substitutions: Vec::new(),
        directory_entry: "index".into(),
    }
}

fn module_graph() -> ModuleGraph {
    let mut graph = ModuleGraph::default();
    for path in INDEXED {
        graph.inputs.insert(
            (*path).to_string(),
            ModuleInput {
                path: (*path).to_string(),
                paths: source_rules(),
                ..Default::default()
            },
        );
    }
    graph.rebuild(&Lookup::new());
    graph
}

/// A workspace whose only declared package is `core_client`, spelled the way
/// its ecosystem spells an import of it. `source_root` is the project-relative
/// directory that package publishes from, when it declares one.
fn workspace(source_root: Option<&str>) -> Compilation {
    let mut compilation = Compilation::empty(Arc::new(TypeArena::new()));
    compilation.modules = module_graph();
    compilation.workspace_pkg_by_declared_name = [("package:core_client".to_string(), 1)]
        .into_iter()
        .collect();
    compilation.module_specifier.pkg_source_root = source_root
        .map(|root| (1_i64, root.to_string()))
        .into_iter()
        .collect();
    compilation
}

const IMPORTER: &str = "packages/app_server/lib/endpoint.dart";

#[test]
fn deep_specifier_maps_through_the_declared_source_root() {
    let compilation = workspace(Some("packages/core_client/lib"));
    assert_eq!(
        compilation.workspace_source_root_entry(IMPORTER, "package:core_client/core_client.dart"),
        Some("packages/core_client/lib/core_client.dart")
    );
    assert_eq!(
        compilation.workspace_source_root_entry(IMPORTER, "package:core_client/src/codec.dart"),
        Some("packages/core_client/lib/src/codec.dart")
    );
}

#[test]
fn a_package_with_no_declared_source_root_is_left_to_the_other_links() {
    let compilation = workspace(None);
    assert_eq!(
        compilation.workspace_source_root_entry(IMPORTER, "package:core_client/core_client.dart"),
        None
    );
}

#[test]
fn the_bare_package_spelling_names_no_file_of_its_own() {
    let compilation = workspace(Some("packages/core_client/lib"));
    assert_eq!(
        compilation.workspace_source_root_entry(IMPORTER, "package:core_client"),
        None
    );
}

#[test]
fn an_undeclared_package_head_maps_nothing() {
    let compilation = workspace(Some("packages/core_client/lib"));
    assert_eq!(
        compilation.workspace_source_root_entry(IMPORTER, "package:other_pkg/core_client.dart"),
        None
    );
}
