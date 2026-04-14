use crate::types::*;

pub static MARKDOWN: LanguageDescriptor = LanguageDescriptor {
    id: "markdown",
    display_name: "Markdown",
    file_extensions: &[".md", ".markdown", ".mdown", ".mkd", ".mkdn", ".mdx"],
    filenames: &["README", "CHANGELOG", "CONTRIBUTING", "LICENSE"],
    aliases: &["md"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: None,
};
