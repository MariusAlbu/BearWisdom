use crate::types::*;
pub static SYSTEMD: LanguageDescriptor = LanguageDescriptor {
    id: "systemd", display_name: "systemd Unit",
    file_extensions: &[".service", ".timer", ".socket", ".path", ".target", ".mount", ".automount"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: Some("#"), block_comment: None,
};
