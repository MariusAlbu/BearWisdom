use crate::types::*;

pub static MATLAB: LanguageDescriptor = LanguageDescriptor {
    id: "matlab",
    display_name: "MATLAB",
    // .m added — BearWisdom has no Objective-C extractor, so no conflict.
    // .mat is a binary format but kept for completeness.
    file_extensions: &[".m", ".mat"],
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
