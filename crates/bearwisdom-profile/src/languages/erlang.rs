use crate::types::*;

pub static ERLANG: LanguageDescriptor = LanguageDescriptor {
    id: "erlang",
    display_name: "Erlang",
    file_extensions: &[".erl", ".hrl"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("%"),
    block_comment: None,
};
