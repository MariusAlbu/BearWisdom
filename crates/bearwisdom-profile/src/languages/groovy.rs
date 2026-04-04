use crate::types::*;

pub static GROOVY: LanguageDescriptor = LanguageDescriptor {
    id: "groovy",
    display_name: "Groovy",
    file_extensions: &[".groovy", ".gradle"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
