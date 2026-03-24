use crate::types::*;

static GO_PM: PmDescriptor = PmDescriptor {
    name: "go modules",
    lock_file: Some("go.sum"),
    deps_dir: None, // managed in GOPATH/pkg/mod, not local
    install_cmd: ShellCommands::same("go mod download"),
    restore_cmd: ShellCommands::same("go mod download"),
};

static GO_TEST: TfDescriptor = TfDescriptor {
    name: "go-test",
    display_name: "go test",
    config_files: &["go.mod"],
    config_content_match: None,
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("go test ./... -list .")),
    run_cmd: ShellCommands::same("go test ./..."),
    run_single_cmd: ShellCommands::same("go test {file}"),
};

static GO_FETCH: RestoreStep = RestoreStep {
    id: "go-mod-download",
    title: "Download Go modules",
    description: "Run `go mod download` to fetch all module dependencies.",
    trigger: RestoreTrigger::FileMissing,
    watch_path: "go.sum",
    commands: ShellCommands::same("go mod download"),
    auto_fixable: true,
    critical: true,
};

pub static GO: LanguageDescriptor = LanguageDescriptor {
    id: "go",
    display_name: "Go",
    file_extensions: &[".go"],
    filenames: &[],
    aliases: &["golang"],
    exclude_dirs: &["vendor"],
    entry_point_files: &["go.mod", "go.sum"],
    sdk: Some(SdkDescriptor {
        name: "Go",
        version_command: "go",
        version_args: &["version"],
        version_file: Some("go.mod"),
        version_json_key: None,
        install_url: "https://go.dev/dl/",
    }),
    package_managers: &[GO_PM],
    test_frameworks: &[GO_TEST],
    restore_steps: &[GO_FETCH],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
