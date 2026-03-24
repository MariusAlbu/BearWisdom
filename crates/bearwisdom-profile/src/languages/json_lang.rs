use crate::types::*;

pub static JSON: LanguageDescriptor = LanguageDescriptor {
    id: "json",
    display_name: "JSON",
    file_extensions: &[".json", ".jsonc", ".json5"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None, // JSONC supports // but plain JSON does not
    block_comment: None,
};
