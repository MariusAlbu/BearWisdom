use crate::types::*;

pub static ASTRO: LanguageDescriptor = LanguageDescriptor {
    id: "astro",
    display_name: "Astro",
    file_extensions: &[".astro"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[".astro", "dist"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
