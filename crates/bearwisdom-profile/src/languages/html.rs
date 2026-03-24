use crate::types::*;

pub static HTML: LanguageDescriptor = LanguageDescriptor {
    id: "html",
    display_name: "HTML",
    file_extensions: &[".html", ".htm", ".xhtml"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &["index.html"],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("<!--", "-->")),
};
