use crate::types::*;

pub static SVELTE: LanguageDescriptor = LanguageDescriptor {
    id: "svelte",
    display_name: "Svelte",
    file_extensions: &[".svelte"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[".svelte-kit"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
