use crate::types::*;

pub static ANGULAR_TEMPLATE: LanguageDescriptor = LanguageDescriptor {
    id: "angular_template",
    display_name: "Angular Template",
    file_extensions: &[".component.html", ".container.html", ".dialog.html"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("<!--", "-->")),
};
