use crate::types::*;
pub static JSP: LanguageDescriptor = LanguageDescriptor {
    id: "jsp", display_name: "JSP",
    file_extensions: &[".jsp", ".jspx", ".tag"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: Some(("<%--", "--%>")),
};
