use crate::types::*;
pub static FREEMARKER: LanguageDescriptor = LanguageDescriptor {
    id: "freemarker", display_name: "FreeMarker",
    file_extensions: &[".ftl", ".ftlh", ".ftlx"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: Some(("<#--", "-->")),
};
