use crate::types::*;

pub static HCL: LanguageDescriptor = LanguageDescriptor {
    id: "hcl",
    display_name: "HCL",
    file_extensions: &[".tf", ".tfvars", ".hcl"],
    filenames: &[],
    aliases: &["terraform"],
    exclude_dirs: &[".terraform"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: Some(("/*", "*/")),
};
