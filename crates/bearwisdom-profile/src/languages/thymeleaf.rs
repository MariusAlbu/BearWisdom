use crate::types::*;
pub static THYMELEAF: LanguageDescriptor = LanguageDescriptor {
    id: "thymeleaf", display_name: "Thymeleaf",
    // `.th.html` is the explicit form. Plain `.html` in Spring templates/
    // directories is resolved via HTML plugin; path-based disambiguation
    // is a future enhancement.
    file_extensions: &[".th.html"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: Some(("<!--/*", "*/-->")),
};
