use crate::types::*;

pub static PUPPET: LanguageDescriptor = LanguageDescriptor {
    id: "puppet",
    display_name: "Puppet",
    file_extensions: &[".pp"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: Some(("/*", "*/")),
};
