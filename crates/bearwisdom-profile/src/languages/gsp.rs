use crate::types::*;
pub static GSP: LanguageDescriptor = LanguageDescriptor {
    id: "gsp", display_name: "Groovy Server Pages",
    file_extensions: &[".gsp"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: Some(("<%--", "--%>")),
};
