use crate::types::*;

pub static ROBOT: LanguageDescriptor = LanguageDescriptor {
    id: "robot",
    display_name: "Robot Framework",
    // .resource files share Robot Framework syntax — they're shared
    // keyword libraries written in the same `*** Keywords ***` shape as
    // `.robot` test suites. Without indexing them every BDD-style test
    // suite that imports a `.resource` keyword library lands the calls
    // in unresolved_refs.
    file_extensions: &[".robot", ".resource"],
    filenames: &[],
    aliases: &["robotframework"],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
