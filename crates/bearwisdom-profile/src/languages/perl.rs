use crate::types::*;

pub static PERL: LanguageDescriptor = LanguageDescriptor {
    id: "perl",
    display_name: "Perl",
    // .pl omitted — conflicts with Prolog. .pm is unambiguous.
    file_extensions: &[".pm"],
    filenames: &["cpanfile"],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
