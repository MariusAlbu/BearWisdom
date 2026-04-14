use crate::types::*;

pub static NUNJUCKS: LanguageDescriptor = LanguageDescriptor {
    id: "nunjucks",
    display_name: "Nunjucks",
    file_extensions: &[".njk", ".nunjucks"],
    filenames: &[],
    aliases: &["njk"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("{#", "#}")),
};
