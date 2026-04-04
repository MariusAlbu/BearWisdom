use crate::types::*;

pub static FORTRAN: LanguageDescriptor = LanguageDescriptor {
    id: "fortran",
    display_name: "Fortran",
    file_extensions: &[".f90", ".f95", ".f03", ".f08"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("!"),
    block_comment: None,
};
