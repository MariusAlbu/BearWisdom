use crate::types::*;

pub static BLADE: LanguageDescriptor = LanguageDescriptor {
    id: "blade",
    display_name: "Blade",
    file_extensions: &[".blade.php"],
    filenames: &[],
    aliases: &["laravel-blade"],
    exclude_dirs: &["vendor", "node_modules", "storage"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("{{--", "--}}")),
};
