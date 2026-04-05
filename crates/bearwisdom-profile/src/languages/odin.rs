use crate::types::*;

pub static ODIN: LanguageDescriptor = LanguageDescriptor {
    id: "odin",
    display_name: "Odin",
    file_extensions: &[".odin"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
