use super::WorkspacePackageResolver;
use crate::indexer::module_resolution::ModuleResolver;

fn resolver() -> WorkspacePackageResolver {
    WorkspacePackageResolver::new(vec![
        ("next".to_string(), "packages/next".to_string()),
        ("@tryghost/logging".to_string(), "ghost/logging".to_string()),
    ])
}

#[test]
fn deep_import_maps_into_the_package_root() {
    let files = ["packages/next/link.js", "packages/next/image.js"];
    assert_eq!(
        resolver().resolve_to_file("next/link", "docs/a.mdx", &files),
        Some("packages/next/link.js".to_string())
    );
}

#[test]
fn declaration_double_extension_serves_the_sub_path() {
    let files = ["packages/next/types/link.d.ts", "packages/next/link.d.ts"];
    assert_eq!(
        resolver().resolve_to_file("next/link", "src/a.tsx", &files),
        Some("packages/next/link.d.ts".to_string())
    );
}

#[test]
fn sub_path_index_file_serves_the_directory_form() {
    let files = ["packages/next/dist/server/index.js"];
    assert_eq!(
        resolver().resolve_to_file("next/dist/server", "src/a.ts", &files),
        Some("packages/next/dist/server/index.js".to_string())
    );
}

#[test]
fn bare_package_resolves_only_to_a_root_index() {
    let files = ["packages/next/index.ts", "packages/next/link.js"];
    assert_eq!(
        resolver().resolve_to_file("next", "src/a.ts", &files),
        Some("packages/next/index.ts".to_string())
    );
    let no_index = ["packages/next/link.js"];
    assert_eq!(resolver().resolve_to_file("next", "src/a.ts", &no_index), None);
}

#[test]
fn scoped_name_matches_before_shorter_heads() {
    let files = ["ghost/logging/index.js"];
    assert_eq!(
        resolver().resolve_to_file("@tryghost/logging", "app.js", &files),
        Some("ghost/logging/index.js".to_string())
    );
}

#[test]
fn qualified_separator_normalizes_to_slash() {
    let r = WorkspacePackageResolver::new(vec![(
        "tantivy".to_string(),
        "crates/tantivy".to_string(),
    )]);
    let files = ["crates/tantivy/schema.rs"];
    assert_eq!(
        r.resolve_to_file("tantivy::schema", "src/main.rs", &files),
        Some("crates/tantivy/schema.rs".to_string())
    );
}

#[test]
fn unknown_head_and_relative_specs_decline() {
    let files = ["packages/next/link.js"];
    assert_eq!(resolver().resolve_to_file("react/jsx", "a.ts", &files), None);
    assert_eq!(resolver().resolve_to_file("./link", "a.ts", &files), None);
}
