use crate::types::*;

pub static POLYGLOT_NB: LanguageDescriptor = LanguageDescriptor {
    id: "polyglot_nb",
    display_name: ".NET Polyglot Notebook",
    file_extensions: &[".dib", ".dotnet-interactive"],
    filenames: &[],
    aliases: &["dib"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: None,
};
