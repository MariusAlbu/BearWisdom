use crate::types::*;

pub static VBNET: LanguageDescriptor = LanguageDescriptor {
    id: "vbnet",
    display_name: "VB.NET",
    file_extensions: &[".vb"],
    filenames: &[],
    aliases: &["vb", "visualbasic"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("'"),
    block_comment: None,
};
