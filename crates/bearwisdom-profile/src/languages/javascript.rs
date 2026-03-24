use crate::types::*;

// JavaScript shares the Node.js ecosystem. PmDescriptor and TfDescriptor constants
// are re-declared here as separate statics (can't share across crate statics directly
// without indirection, but values are identical — zero runtime cost).

static NPM: PmDescriptor = PmDescriptor {
    name: "npm",
    lock_file: Some("package-lock.json"),
    deps_dir: Some("node_modules"),
    install_cmd: ShellCommands::same("npm install"),
    restore_cmd: ShellCommands::same("npm ci"),
};

static PNPM: PmDescriptor = PmDescriptor {
    name: "pnpm",
    lock_file: Some("pnpm-lock.yaml"),
    deps_dir: Some("node_modules"),
    install_cmd: ShellCommands::same("pnpm install"),
    restore_cmd: ShellCommands::same("pnpm install --frozen-lockfile"),
};

static YARN: PmDescriptor = PmDescriptor {
    name: "yarn",
    lock_file: Some("yarn.lock"),
    deps_dir: Some("node_modules"),
    install_cmd: ShellCommands::same("yarn install"),
    restore_cmd: ShellCommands::same("yarn install --frozen-lockfile"),
};

static JEST: TfDescriptor = TfDescriptor {
    name: "jest",
    display_name: "Jest",
    config_files: &["jest.config.js", "jest.config.json"],
    config_content_match: None,
    package_json_dep: Some("jest"),
    discovery_cmd: Some(ShellCommands::same("npx jest --listTests")),
    run_cmd: ShellCommands::same("npx jest"),
    run_single_cmd: ShellCommands::same("npx jest {file}"),
};

static MOCHA: TfDescriptor = TfDescriptor {
    name: "mocha",
    display_name: "Mocha",
    config_files: &[".mocharc.js", ".mocharc.json", ".mocharc.yml"],
    config_content_match: None,
    package_json_dep: Some("mocha"),
    discovery_cmd: None,
    run_cmd: ShellCommands::same("npx mocha"),
    run_single_cmd: ShellCommands::same("npx mocha {file}"),
};

static NODE_MODULES_RESTORE: RestoreStep = RestoreStep {
    id: "npm-install-js",
    title: "Install Node.js dependencies",
    description: "node_modules is missing. Run the package manager install command.",
    trigger: RestoreTrigger::DirMissing,
    watch_path: "node_modules",
    commands: ShellCommands::same("npm install"),
    auto_fixable: true,
    critical: true,
};

pub static JAVASCRIPT: LanguageDescriptor = LanguageDescriptor {
    id: "javascript",
    display_name: "JavaScript",
    file_extensions: &[".js", ".jsx", ".mjs", ".cjs"],
    filenames: &[],
    aliases: &["js", "jsx"],
    exclude_dirs: &["node_modules", ".next", ".nuxt", ".output", "dist"],
    entry_point_files: &["package.json", "package-lock.json"],
    sdk: Some(SdkDescriptor {
        name: "Node.js",
        version_command: "node",
        version_args: &["--version"],
        version_file: Some(".nvmrc"),
        version_json_key: None,
        install_url: "https://nodejs.org",
    }),
    package_managers: &[NPM, PNPM, YARN],
    test_frameworks: &[JEST, MOCHA],
    restore_steps: &[NODE_MODULES_RESTORE],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
