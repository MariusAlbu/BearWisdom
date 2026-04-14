use crate::types::*;

pub static RAZOR: LanguageDescriptor = LanguageDescriptor {
    id: "razor",
    display_name: "Razor",
    file_extensions: &[".cshtml", ".razor"],
    filenames: &[],
    aliases: &["cshtml"],
    exclude_dirs: &["bin", "obj"],
    entry_point_files: &["_ViewImports.cshtml", "_Layout.cshtml"],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("@*", "*@")),
};
