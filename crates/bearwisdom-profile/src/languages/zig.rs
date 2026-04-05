use crate::types::*;

pub static ZIG: LanguageDescriptor = LanguageDescriptor {
    id: "zig",
    display_name: "Zig",
    file_extensions: &[".zig", ".zon"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &["zig-cache", "zig-out"],
    entry_point_files: &["build.zig"],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: None,
};
