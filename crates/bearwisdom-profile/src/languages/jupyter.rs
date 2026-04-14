use crate::types::*;

pub static JUPYTER: LanguageDescriptor = LanguageDescriptor {
    id: "jupyter",
    display_name: "Jupyter Notebook",
    file_extensions: &[".ipynb"],
    filenames: &[],
    aliases: &["ipynb"],
    exclude_dirs: &[".ipynb_checkpoints"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: None,
};
