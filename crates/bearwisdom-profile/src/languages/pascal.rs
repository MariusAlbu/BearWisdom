use crate::types::*;

pub static PASCAL: LanguageDescriptor = LanguageDescriptor {
    id: "pascal",
    display_name: "Pascal",
    // .pp omitted — conflicts with Puppet (Puppet is more common).
    file_extensions: &[".pas", ".dpr"],
    filenames: &[],
    aliases: &["delphi"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("{", "}")),
};
