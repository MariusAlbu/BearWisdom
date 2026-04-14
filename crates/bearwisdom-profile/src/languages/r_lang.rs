use crate::types::*;

pub static R: LanguageDescriptor = LanguageDescriptor {
    id: "r",
    display_name: "R",
    file_extensions: &[".R", ".r"],
    filenames: &[],
    aliases: &["rlang"],
    exclude_dirs: &[".Rproj.user", "renv"],
    entry_point_files: &["DESCRIPTION", "NAMESPACE", "renv.lock"],
    sdk: Some(SdkDescriptor {
        name: "R",
        version_command: "Rscript",
        version_args: &["--version"],
        version_file: None,
        version_json_key: None,
        install_url: "https://www.r-project.org/",
    }),
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
