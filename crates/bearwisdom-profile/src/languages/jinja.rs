use crate::types::*;

pub static JINJA: LanguageDescriptor = LanguageDescriptor {
    id: "jinja",
    display_name: "Jinja2",
    file_extensions: &[".jinja", ".jinja2", ".j2"],
    filenames: &[],
    aliases: &["j2", "jinja2"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("{#", "#}")),
};
