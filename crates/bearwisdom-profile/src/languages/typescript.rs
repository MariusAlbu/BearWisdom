use crate::types::*;

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

static BUN: PmDescriptor = PmDescriptor {
    name: "bun",
    lock_file: Some("bun.lockb"),
    deps_dir: Some("node_modules"),
    install_cmd: ShellCommands::same("bun install"),
    restore_cmd: ShellCommands::same("bun install --frozen-lockfile"),
};

static VITEST: TfDescriptor = TfDescriptor {
    name: "vitest",
    display_name: "Vitest",
    config_files: &["vitest.config.ts", "vitest.config.js", "vite.config.ts"],
    config_content_match: Some("vitest"),
    package_json_dep: Some("vitest"),
    discovery_cmd: Some(ShellCommands::same("npx vitest list")),
    run_cmd: ShellCommands::same("npx vitest run"),
    run_single_cmd: ShellCommands::same("npx vitest run {file}"),
};

static JEST: TfDescriptor = TfDescriptor {
    name: "jest",
    display_name: "Jest",
    config_files: &["jest.config.ts", "jest.config.js", "jest.config.json"],
    config_content_match: None,
    package_json_dep: Some("jest"),
    discovery_cmd: Some(ShellCommands::same("npx jest --listTests")),
    run_cmd: ShellCommands::same("npx jest"),
    run_single_cmd: ShellCommands::same("npx jest {file}"),
};

static PLAYWRIGHT: TfDescriptor = TfDescriptor {
    name: "playwright",
    display_name: "Playwright",
    config_files: &["playwright.config.ts", "playwright.config.js"],
    config_content_match: None,
    package_json_dep: Some("@playwright/test"),
    discovery_cmd: Some(ShellCommands::same("npx playwright test --list")),
    run_cmd: ShellCommands::same("npx playwright test"),
    run_single_cmd: ShellCommands::same("npx playwright test {file}"),
};

static CYPRESS: TfDescriptor = TfDescriptor {
    name: "cypress",
    display_name: "Cypress",
    config_files: &["cypress.config.ts", "cypress.config.js"],
    config_content_match: None,
    package_json_dep: Some("cypress"),
    discovery_cmd: None,
    run_cmd: ShellCommands::same("npx cypress run"),
    run_single_cmd: ShellCommands::same("npx cypress run --spec {file}"),
};

static MOCHA: TfDescriptor = TfDescriptor {
    name: "mocha",
    display_name: "Mocha",
    config_files: &[".mocharc.js", ".mocharc.ts", ".mocharc.json", ".mocharc.yml"],
    config_content_match: None,
    package_json_dep: Some("mocha"),
    discovery_cmd: None,
    run_cmd: ShellCommands::same("npx mocha"),
    run_single_cmd: ShellCommands::same("npx mocha {file}"),
};

static NODE_MODULES_RESTORE: RestoreStep = RestoreStep {
    id: "npm-install",
    title: "Install Node.js dependencies",
    description: "node_modules is missing. Run the package manager install command.",
    trigger: RestoreTrigger::DirMissing,
    watch_path: "node_modules",
    commands: ShellCommands::same("npm install"),
    auto_fixable: true,
    critical: true,
};

pub static TYPESCRIPT: LanguageDescriptor = LanguageDescriptor {
    id: "typescript",
    display_name: "TypeScript",
    file_extensions: &[".ts", ".tsx", ".mts", ".cts"],
    filenames: &[],
    aliases: &["ts", "tsx"],
    exclude_dirs: &["node_modules", ".next", ".nuxt", ".output", ".svelte-kit", "dist", ".turbo"],
    entry_point_files: &["tsconfig.json", "package.json", "package-lock.json", "pnpm-lock.yaml", "yarn.lock"],
    sdk: Some(SdkDescriptor {
        name: "Node.js",
        version_command: "node",
        version_args: &["--version"],
        version_file: Some(".nvmrc"),
        version_json_key: None,
        install_url: "https://nodejs.org",
    }),
    package_managers: &[NPM, PNPM, YARN, BUN],
    test_frameworks: &[VITEST, JEST, PLAYWRIGHT, CYPRESS, MOCHA],
    restore_steps: &[NODE_MODULES_RESTORE],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
