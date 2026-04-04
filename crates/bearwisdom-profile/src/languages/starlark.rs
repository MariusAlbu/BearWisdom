use crate::types::*;

pub static STARLARK: LanguageDescriptor = LanguageDescriptor {
    id: "starlark",
    display_name: "Starlark",
    file_extensions: &[".bzl", ".star"],
    filenames: &["BUILD", "WORKSPACE"],
    aliases: &["bazel", "bzl"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
