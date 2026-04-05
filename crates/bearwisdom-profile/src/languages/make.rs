use crate::types::*;

pub static MAKE: LanguageDescriptor = LanguageDescriptor {
    id: "make",
    display_name: "Make",
    file_extensions: &[".mk", ".mak"],
    filenames: &["Makefile", "GNUmakefile", "makefile"],
    aliases: &["makefile"],
    exclude_dirs: &[],
    entry_point_files: &["Makefile"],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
