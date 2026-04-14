use crate::types::*;
pub static EEX: LanguageDescriptor = LanguageDescriptor {
    id: "eex", display_name: "EEx",
    file_extensions: &[".eex", ".leex", ".html.eex"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: Some(("<%#", "%>")),
};
