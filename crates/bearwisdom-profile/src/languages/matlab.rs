use crate::types::*;

pub static MATLAB: LanguageDescriptor = LanguageDescriptor {
    id: "matlab",
    display_name: "MATLAB",
    // .m omitted — conflicts with Objective-C (ObjC is more common in existing C extractor).
    // .mat is a binary format, not source. Detection relies on filenames heuristic only.
    file_extensions: &[".mat"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("%"),
    block_comment: Some(("%{", "%}")),
};
