use crate::types::*;
pub static NGINX: LanguageDescriptor = LanguageDescriptor {
    id: "nginx", display_name: "Nginx",
    file_extensions: &[".nginx"],
    filenames: &["nginx.conf"], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: Some("#"), block_comment: None,
};
