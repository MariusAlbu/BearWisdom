use crate::types::*;

pub static PERL: LanguageDescriptor = LanguageDescriptor {
    id: "perl",
    display_name: "Perl",
    // .pl is listed first so the walker finds it before Prolog's .pro/.P extensions.
    // The registry iterates LANGUAGES in order; Perl appears before Prolog, so .pl
    // is claimed by Perl. The Prolog descriptor intentionally omits .pl.
    file_extensions: &[".pl", ".pm"],
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
