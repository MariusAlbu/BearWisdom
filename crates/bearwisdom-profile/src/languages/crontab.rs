use crate::types::*;
pub static CRONTAB: LanguageDescriptor = LanguageDescriptor {
    id: "crontab", display_name: "Crontab",
    file_extensions: &[".cron", ".crontab"],
    filenames: &["crontab"], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: Some("#"), block_comment: None,
};
