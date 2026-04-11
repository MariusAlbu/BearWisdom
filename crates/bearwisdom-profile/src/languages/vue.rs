use crate::types::*;

pub static VUE: LanguageDescriptor = LanguageDescriptor {
    id: "vue",
    display_name: "Vue",
    file_extensions: &[".vue"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[".nuxt", ".output"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
