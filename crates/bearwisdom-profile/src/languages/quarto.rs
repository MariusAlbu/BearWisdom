use crate::types::*;

pub static QUARTO: LanguageDescriptor = LanguageDescriptor {
    id: "quarto",
    display_name: "Quarto",
    file_extensions: &[".qmd"],
    filenames: &[],
    aliases: &["qmd"],
    exclude_dirs: &["_site", "_freeze"],
    entry_point_files: &["_quarto.yml"],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: None,
};
