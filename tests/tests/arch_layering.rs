//! Architecture layering constraints over the workspace crate graph.
//!
//! Two invariants are frozen here:
//! 1. the core library `bearwisdom` depends on no binary/application or test crate;
//! 2. `tree-sitter-*` grammar crates are used only by the grammar-aggregation and
//!    highlighting library layer, never by the application, benchmark, or test crates.

use cargo_metadata::MetadataCommand;

/// Crates the core library `bearwisdom` must never depend on: the binaries, the
/// benchmark harnesses, and the integration-test crate.
const FORBIDDEN_FOR_CORE: &[&str] = &[
    "bearwisdom-cli",
    "bearwisdom-mcp",
    "bearwisdom-web",
    "bearwisdom-bench",
    "bw-bench",
    "bearwisdom-tests",
];

/// Workspace crates permitted to depend on `tree-sitter-*` grammar crates.
const GRAMMAR_DEPENDENTS: &[&str] = &[
    "bearwisdom",
    "code-grammars",
    "code-highlight",
    "tree-sitter-dockerfile-0-25",
    "tree-sitter-scss-local",
];

/// True for a tree-sitter grammar crate. `tree-sitter`, `tree-sitter-language`,
/// and `tree-sitter-highlight` are support libraries, not grammars, so callers
/// may depend on them anywhere.
fn is_grammar_crate(name: &str) -> bool {
    name.starts_with("tree-sitter-")
        && name != "tree-sitter-language"
        && name != "tree-sitter-highlight"
}

fn workspace_metadata() -> cargo_metadata::Metadata {
    MetadataCommand::new()
        .no_deps()
        .exec()
        .expect("cargo metadata must succeed")
}

#[test]
fn core_does_not_depend_on_binaries_or_tests() {
    let metadata = workspace_metadata();
    let core = metadata
        .packages
        .iter()
        .find(|p| p.name.as_str() == "bearwisdom")
        .expect("workspace contains the core crate `bearwisdom`");

    let mut violations: Vec<String> = core
        .dependencies
        .iter()
        .filter(|dep| FORBIDDEN_FOR_CORE.contains(&dep.name.as_str()))
        .map(|dep| format!("bearwisdom -> {}", dep.name))
        .collect();
    violations.sort();

    assert!(
        violations.is_empty(),
        "core crate `bearwisdom` must not depend on binary/application or test crates; \
         forbidden edges present: {violations:?}",
    );
}

#[test]
fn grammar_crates_stay_in_the_library_layer() {
    let metadata = workspace_metadata();

    let mut violations: Vec<String> = Vec::new();
    for pkg in &metadata.packages {
        if GRAMMAR_DEPENDENTS.contains(&pkg.name.as_str()) {
            continue;
        }
        for dep in &pkg.dependencies {
            if is_grammar_crate(&dep.name) {
                violations.push(format!("{} -> {}", pkg.name, dep.name));
            }
        }
    }
    violations.sort();

    assert!(
        violations.is_empty(),
        "tree-sitter grammar crates may be depended on only by {GRAMMAR_DEPENDENTS:?}; \
         forbidden edges present: {violations:?}",
    );
}
