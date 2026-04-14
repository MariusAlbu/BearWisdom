use crate::types::*;

pub static RMARKDOWN: LanguageDescriptor = LanguageDescriptor {
    id: "rmarkdown",
    display_name: "RMarkdown",
    file_extensions: &[".Rmd", ".rmd"],
    filenames: &[],
    aliases: &["rmd"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: None,
};
