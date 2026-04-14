use crate::types::*;
pub static SMARTY: LanguageDescriptor = LanguageDescriptor {
    id: "smarty", display_name: "Smarty",
    // `.tpl` collides with Go templates; require `.smarty.tpl` or
    // `.smarty` to disambiguate in the MVP. Real Smarty codebases
    // conventionally just use `.tpl` — detection via path heuristic
    // (presence of `smarty/` directory or `Smarty.class.php`) is a
    // future enhancement.
    file_extensions: &[".smarty", ".smarty.tpl"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: Some(("{*", "*}")),
};
