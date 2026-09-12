// =============================================================================
// module_relative_resolution_tests — relative specifiers land on indexed files
// =============================================================================

use crate::indexer::resolve::engine::{
    module_graph::ModuleGraph, module_input::ModuleInput, module_paths::PathRules, testkit::Lookup,
};

fn source_rules() -> PathRules {
    PathRules {
        extensions: vec![".ts".into(), ".tsx".into()],
        substitutions: vec![(".js".into(), vec![".ts".into(), ".tsx".into()])],
        directory_entry: "index".into(),
    }
}

fn input(path: &str) -> ModuleInput {
    ModuleInput {
        path: path.into(),
        paths: source_rules(),
        ..Default::default()
    }
}

fn graph() -> ModuleGraph {
    let mut graph = ModuleGraph::default();
    for path in [
        "src/article/article.entity.ts",
        "src/user/user.entity.ts",
        "src/tag/index.ts",
        "src/store.ts",
    ] {
        graph.inputs.insert(path.into(), input(path));
    }
    graph.rebuild(&Lookup::new());
    graph
}

#[test]
fn parent_relative_sibling_and_directory_entry_specifiers_resolve_by_the_source_rules() {
    let graph = graph();
    let from = "src/article/article.entity.ts";
    assert_eq!(
        graph.resolve_relative(from, "../user/user.entity"),
        Some("src/user/user.entity.ts")
    );
    assert_eq!(graph.resolve_relative(from, "../tag"), Some("src/tag/index.ts"));
    assert_eq!(
        graph.resolve_relative(from, "../store.js"),
        Some("src/store.ts"),
        "an emitted extension substitutes to its source"
    );
}

#[test]
fn bare_specifiers_unknown_sources_and_unspelled_bases_resolve_to_nothing() {
    let graph = graph();
    let from = "src/article/article.entity.ts";
    assert_eq!(graph.resolve_relative(from, "react"), None);
    assert_eq!(graph.resolve_relative(from, "../user/missing"), None);
    assert_eq!(
        graph.resolve_relative("src/unknown.ts", "./store"),
        None,
        "a source without a module input supplies no path rules"
    );
}

#[test]
fn an_alias_target_resolves_as_a_project_relative_base() {
    let graph = graph();
    let from = "src/article/article.entity.ts";
    assert_eq!(graph.resolve_base(from, "src/tag"), Some("src/tag/index.ts"));
    assert_eq!(graph.resolve_base(from, "src/store"), Some("src/store.ts"));
    assert_eq!(graph.resolve_base(from, "src/missing"), None);
}
