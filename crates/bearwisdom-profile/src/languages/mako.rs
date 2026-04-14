use crate::types::*;
pub static MAKO: LanguageDescriptor = LanguageDescriptor {
    id: "mako", display_name: "Mako",
    file_extensions: &[".mako", ".html.mako"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: Some(("<%doc>", "</%doc>")),
};
