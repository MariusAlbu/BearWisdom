use crate::types::*;

pub static VBA: LanguageDescriptor = LanguageDescriptor {
    id: "vba",
    display_name: "VBA",
    file_extensions: &[".bas", ".cls", ".frm"],
    filenames: &[],
    aliases: &["visual basic for applications"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("'"),
    block_comment: None,
};
