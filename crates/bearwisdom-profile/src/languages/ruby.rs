use crate::types::*;

static BUNDLER: PmDescriptor = PmDescriptor {
    name: "bundler",
    lock_file: Some("Gemfile.lock"),
    deps_dir: Some("vendor/bundle"),
    install_cmd: ShellCommands::same("bundle install"),
    restore_cmd: ShellCommands::same("bundle install"),
};

static RSPEC: TfDescriptor = TfDescriptor {
    name: "rspec",
    display_name: "RSpec",
    config_files: &[".rspec", "spec/spec_helper.rb"],
    config_content_match: None,
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("bundle exec rspec --dry-run --format json")),
    run_cmd: ShellCommands::same("bundle exec rspec"),
    run_single_cmd: ShellCommands::same("bundle exec rspec {file}"),
};

static MINITEST: TfDescriptor = TfDescriptor {
    name: "minitest",
    display_name: "Minitest",
    config_files: &["test/test_helper.rb"],
    config_content_match: Some("Minitest"),
    package_json_dep: None,
    discovery_cmd: None,
    run_cmd: ShellCommands::same("bundle exec rake test"),
    run_single_cmd: ShellCommands::same("bundle exec ruby -Ilib -Itest {file}"),
};

static BUNDLE_INSTALL: RestoreStep = RestoreStep {
    id: "bundle-install",
    title: "Install Ruby gems",
    description: "Run `bundle install` to install all gems from Gemfile.lock.",
    trigger: RestoreTrigger::FileMissing,
    watch_path: "Gemfile.lock",
    commands: ShellCommands::same("bundle install"),
    auto_fixable: true,
    critical: true,
};

pub static RUBY: LanguageDescriptor = LanguageDescriptor {
    id: "ruby",
    display_name: "Ruby",
    file_extensions: &[".rb", ".rake", ".gemspec", ".ru"],
    filenames: &["Rakefile", "Gemfile"],
    aliases: &["rb"],
    exclude_dirs: &["vendor", ".bundle", "tmp", "log"],
    entry_point_files: &["Gemfile", "Gemfile.lock", "Rakefile", ".ruby-version"],
    sdk: Some(SdkDescriptor {
        name: "Ruby",
        version_command: "ruby",
        version_args: &["--version"],
        version_file: Some(".ruby-version"),
        version_json_key: None,
        install_url: "https://www.ruby-lang.org/en/documentation/installation/",
    }),
    package_managers: &[BUNDLER],
    test_frameworks: &[RSPEC, MINITEST],
    restore_steps: &[BUNDLE_INSTALL],
    line_comment: Some("#"),
    block_comment: Some(("=begin", "=end")),
};
