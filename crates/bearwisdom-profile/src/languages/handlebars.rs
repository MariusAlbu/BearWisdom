use crate::types::*;

pub static HANDLEBARS: LanguageDescriptor = LanguageDescriptor {
    id: "handlebars",
    display_name: "Handlebars",
    file_extensions: &[".hbs", ".handlebars", ".mustache"],
    filenames: &[],
    aliases: &["hbs", "mustache"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("{{!", "}}")),
};
