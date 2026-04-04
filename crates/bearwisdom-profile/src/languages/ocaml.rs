use crate::types::*;

pub static OCAML: LanguageDescriptor = LanguageDescriptor {
    id: "ocaml",
    display_name: "OCaml",
    file_extensions: &[".ml", ".mli"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &["_build"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("(*", "*)")),
};
