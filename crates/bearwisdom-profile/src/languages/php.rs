use crate::types::*;

static COMPOSER: PmDescriptor = PmDescriptor {
    name: "composer",
    lock_file: Some("composer.lock"),
    deps_dir: Some("vendor"),
    install_cmd: ShellCommands::same("composer install"),
    restore_cmd: ShellCommands::same("composer install --no-dev"),
};

static PHPUNIT: TfDescriptor = TfDescriptor {
    name: "phpunit",
    display_name: "PHPUnit",
    config_files: &["phpunit.xml", "phpunit.xml.dist"],
    config_content_match: None,
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("./vendor/bin/phpunit --list-tests")),
    run_cmd: ShellCommands::same("./vendor/bin/phpunit"),
    run_single_cmd: ShellCommands::same("./vendor/bin/phpunit {file}"),
};

static PEST: TfDescriptor = TfDescriptor {
    name: "pest",
    display_name: "Pest",
    config_files: &["pest.config.php"],
    config_content_match: None,
    package_json_dep: None,
    discovery_cmd: None,
    run_cmd: ShellCommands::same("./vendor/bin/pest"),
    run_single_cmd: ShellCommands::same("./vendor/bin/pest {file}"),
};

static COMPOSER_INSTALL: RestoreStep = RestoreStep {
    id: "composer-install",
    title: "Install Composer dependencies",
    description: "Run `composer install` to install all PHP packages.",
    trigger: RestoreTrigger::DirMissing,
    watch_path: "vendor",
    commands: ShellCommands::same("composer install"),
    auto_fixable: true,
    critical: true,
};

pub static PHP: LanguageDescriptor = LanguageDescriptor {
    id: "php",
    display_name: "PHP",
    file_extensions: &[".php", ".phtml", ".php3", ".php4", ".php5"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &["vendor"],
    entry_point_files: &["composer.json", "composer.lock"],
    sdk: Some(SdkDescriptor {
        name: "PHP",
        version_command: "php",
        version_args: &["--version"],
        version_file: Some(".php-version"),
        version_json_key: None,
        install_url: "https://www.php.net/downloads",
    }),
    package_managers: &[COMPOSER],
    test_frameworks: &[PHPUNIT, PEST],
    restore_steps: &[COMPOSER_INSTALL],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
