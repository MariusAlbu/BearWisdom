use crate::types::*;

pub static SHELL: LanguageDescriptor = LanguageDescriptor {
    id: "shell",
    display_name: "Shell",
    file_extensions: &[".sh", ".bash", ".zsh", ".fish", ".ksh"],
    filenames: &[".bashrc", ".zshrc", ".bash_profile", ".profile"],
    aliases: &["bash", "zsh", "sh"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
