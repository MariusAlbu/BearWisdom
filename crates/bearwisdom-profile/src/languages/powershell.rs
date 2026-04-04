use crate::types::*;

pub static POWERSHELL: LanguageDescriptor = LanguageDescriptor {
    id: "powershell",
    display_name: "PowerShell",
    file_extensions: &[".ps1", ".psm1", ".psd1"],
    filenames: &[],
    aliases: &["ps1", "pwsh"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: Some(("<#", "#>")),
};
