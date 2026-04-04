use crate::types::*;

pub static FSHARP: LanguageDescriptor = LanguageDescriptor {
    id: "fsharp",
    display_name: "F#",
    file_extensions: &[".fs", ".fsi", ".fsx"],
    filenames: &[],
    aliases: &["fs"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("(*", "*)")),
};
