use crate::types::*;

pub static ERB: LanguageDescriptor = LanguageDescriptor {
    id: "erb",
    display_name: "ERB",
    file_extensions: &[".erb", ".html.erb", ".rhtml"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("<%#", "%>")),
};
