use crate::types::*;

static NUGET: PmDescriptor = PmDescriptor {
    name: "nuget",
    lock_file: Some("packages.lock.json"),
    deps_dir: Some("packages"),
    install_cmd: ShellCommands {
        bash: "dotnet restore",
        powershell: "dotnet restore",
        cmd: "dotnet restore",
    },
    restore_cmd: ShellCommands::same("dotnet restore"),
};

static DOTNET_TEST: TfDescriptor = TfDescriptor {
    name: "dotnet-test",
    display_name: "dotnet test",
    config_files: &["*.csproj", "*.sln"],
    config_content_match: Some("xunit"),
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("dotnet test --list-tests")),
    run_cmd: ShellCommands::same("dotnet test"),
    run_single_cmd: ShellCommands::same("dotnet test --filter {file}"),
};

static NUNIT: TfDescriptor = TfDescriptor {
    name: "nunit",
    display_name: "NUnit",
    config_files: &["*.csproj"],
    config_content_match: Some("nunit"),
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("dotnet test --list-tests")),
    run_cmd: ShellCommands::same("dotnet test"),
    run_single_cmd: ShellCommands::same("dotnet test --filter {file}"),
};

static DOTNET_RESTORE: RestoreStep = RestoreStep {
    id: "dotnet-restore",
    title: "Restore NuGet packages",
    description: "Run `dotnet restore` to download all NuGet dependencies.",
    trigger: RestoreTrigger::FileMissing,
    watch_path: "*.csproj",
    commands: ShellCommands::same("dotnet restore"),
    auto_fixable: true,
    critical: true,
};

pub static CSHARP: LanguageDescriptor = LanguageDescriptor {
    id: "csharp",
    display_name: "C#",
    file_extensions: &[".cs", ".csx"],
    filenames: &[],
    aliases: &["cs", "c#"],
    exclude_dirs: &["bin", "obj", "publish", "artifacts", "packages", ".vs", "TestResults"],
    entry_point_files: &["*.sln", "*.csproj", "global.json", "NuGet.Config", "Directory.Build.props"],
    sdk: Some(SdkDescriptor {
        name: ".NET SDK",
        version_command: "dotnet",
        version_args: &["--version"],
        version_file: Some("global.json"),
        version_json_key: Some("sdk.version"),
        install_url: "https://dot.net",
    }),
    package_managers: &[NUGET],
    test_frameworks: &[DOTNET_TEST, NUNIT],
    restore_steps: &[DOTNET_RESTORE],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
