use crate::types::*;

pub static NIM: LanguageDescriptor = LanguageDescriptor {
    id: "nim",
    display_name: "Nim",
    file_extensions: &[".nim", ".nims"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: Some(("#[", "]#")),
};
