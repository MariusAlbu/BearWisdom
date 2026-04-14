use crate::types::*;

pub static HEEX: LanguageDescriptor = LanguageDescriptor {
    id: "heex",
    display_name: "HEEx",
    file_extensions: &[".heex"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("<%#", "%>")),
};
