use crate::types::*;

pub static ELIXIR: LanguageDescriptor = LanguageDescriptor {
    id: "elixir",
    display_name: "Elixir",
    file_extensions: &[".ex", ".exs"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &["_build", "deps", ".elixir_ls"],
    entry_point_files: &["mix.exs"],
    sdk: Some(SdkDescriptor {
        name: "Elixir",
        version_command: "elixir",
        version_args: &["--version"],
        version_file: None,
        version_json_key: None,
        install_url: "https://elixir-lang.org/install.html",
    }),
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
