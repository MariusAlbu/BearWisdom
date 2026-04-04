use crate::types::*;

pub static NIX: LanguageDescriptor = LanguageDescriptor {
    id: "nix",
    display_name: "Nix",
    file_extensions: &[".nix"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: Some(("/*", "*/")),
};
