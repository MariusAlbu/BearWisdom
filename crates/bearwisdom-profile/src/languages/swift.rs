use crate::types::*;

static SPM: PmDescriptor = PmDescriptor {
    name: "spm",
    lock_file: Some("Package.resolved"),
    deps_dir: Some(".build"),
    install_cmd: ShellCommands::same("swift package resolve"),
    restore_cmd: ShellCommands::same("swift package resolve"),
};

static SWIFT_TEST: TfDescriptor = TfDescriptor {
    name: "swift-test",
    display_name: "swift test",
    config_files: &["Package.swift"],
    config_content_match: Some(".testTarget"),
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("swift test --list-tests")),
    run_cmd: ShellCommands::same("swift test"),
    run_single_cmd: ShellCommands::same("swift test --filter {file}"),
};

static SPM_RESOLVE: RestoreStep = RestoreStep {
    id: "spm-resolve",
    title: "Resolve Swift Package dependencies",
    description: "Run `swift package resolve` to download SPM dependencies.",
    trigger: RestoreTrigger::DirMissing,
    watch_path: ".build",
    commands: ShellCommands::same("swift package resolve"),
    auto_fixable: true,
    critical: true,
};

pub static SWIFT: LanguageDescriptor = LanguageDescriptor {
    id: "swift",
    display_name: "Swift",
    file_extensions: &[".swift"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[".build", "DerivedData"],
    entry_point_files: &["Package.swift", "Package.resolved"],
    sdk: Some(SdkDescriptor {
        name: "Swift",
        version_command: "swift",
        version_args: &["--version"],
        version_file: Some(".swift-version"),
        version_json_key: None,
        install_url: "https://swift.org/download/",
    }),
    package_managers: &[SPM],
    test_frameworks: &[SWIFT_TEST],
    restore_steps: &[SPM_RESOLVE],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
