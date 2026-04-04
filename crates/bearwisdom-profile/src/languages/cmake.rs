use crate::types::*;

pub static CMAKE: LanguageDescriptor = LanguageDescriptor {
    id: "cmake",
    display_name: "CMake",
    file_extensions: &[".cmake"],
    filenames: &["CMakeLists.txt"],
    aliases: &[],
    exclude_dirs: &["CMakeFiles", "cmake-build-debug", "cmake-build-release"],
    entry_point_files: &["CMakeLists.txt"],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
