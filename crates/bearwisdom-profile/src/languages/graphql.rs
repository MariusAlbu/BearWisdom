use crate::types::*;

pub static GRAPHQL: LanguageDescriptor = LanguageDescriptor {
    id: "graphql",
    display_name: "GraphQL",
    file_extensions: &[".graphql", ".gql"],
    filenames: &[],
    aliases: &["gql"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
