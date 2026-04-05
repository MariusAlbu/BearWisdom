use crate::types::*;

pub static PROLOG: LanguageDescriptor = LanguageDescriptor {
    id: "prolog",
    display_name: "Prolog",
    // .pl conflicts with Perl — Perl takes precedence in the registry (listed first).
    // Prolog projects are detected via .pro / .P extensions, or when the project
    // scanner identifies SWI-Prolog / SICStus markers.
    file_extensions: &[".pro", ".P"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("%"),
    block_comment: Some(("/*", "*/")),
};
