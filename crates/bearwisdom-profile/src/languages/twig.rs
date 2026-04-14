use crate::types::*;

pub static TWIG: LanguageDescriptor = LanguageDescriptor {
    id: "twig",
    display_name: "Twig",
    file_extensions: &[".twig", ".html.twig"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &["vendor", "node_modules", "var/cache"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("{#", "#}")),
};
