use crate::types::*;

static CARGO_FETCH: RestoreStep = RestoreStep {
    id: "cargo-fetch",
    title: "Fetch Cargo dependencies",
    description: "Runs `cargo fetch` to download all crate dependencies into the local registry cache.",
    trigger: RestoreTrigger::DirMissing,
    watch_path: "target",
    commands: ShellCommands::same("cargo fetch"),
    auto_fixable: true,
    critical: true,
};

static CARGO_TEST: TfDescriptor = TfDescriptor {
    name: "cargo-test",
    display_name: "cargo test",
    config_files: &["Cargo.toml"],
    config_content_match: Some("[package]"),
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("cargo test -- --list")),
    run_cmd: ShellCommands::same("cargo test"),
    run_single_cmd: ShellCommands::same("cargo test --test {file}"),
};

static CARGO: PmDescriptor = PmDescriptor {
    name: "cargo",
    lock_file: Some("Cargo.lock"),
    deps_dir: None, // managed in ~/.cargo, not a local dir
    install_cmd: ShellCommands::same("cargo build"),
    restore_cmd: ShellCommands::same("cargo fetch"),
};

pub static RUST: LanguageDescriptor = LanguageDescriptor {
    id: "rust",
    display_name: "Rust",
    file_extensions: &[".rs"],
    filenames: &[],
    aliases: &["rs"],
    exclude_dirs: &["target"],
    entry_point_files: &["Cargo.toml", "Cargo.lock"],
    sdk: Some(SdkDescriptor {
        name: "Rust (rustc)",
        version_command: "rustc",
        version_args: &["--version"],
        version_file: Some("rust-toolchain.toml"),
        version_json_key: None,
        install_url: "https://rustup.rs",
    }),
    package_managers: &[CARGO],
    test_frameworks: &[CARGO_TEST],
    restore_steps: &[CARGO_FETCH],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
