use crate::types::*;

pub static PROTO: LanguageDescriptor = LanguageDescriptor {
    id: "proto",
    display_name: "Protocol Buffers",
    file_extensions: &[".proto"],
    filenames: &[],
    aliases: &["protobuf"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
