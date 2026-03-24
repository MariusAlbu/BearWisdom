use crate::types::*;

static GRADLE: PmDescriptor = PmDescriptor {
    name: "gradle",
    lock_file: None,
    deps_dir: None,
    install_cmd: ShellCommands::same("./gradlew build -x test"),
    restore_cmd: ShellCommands::same("./gradlew dependencies"),
};

static MAVEN: PmDescriptor = PmDescriptor {
    name: "maven",
    lock_file: None,
    deps_dir: None,
    install_cmd: ShellCommands::same("mvn install -DskipTests"),
    restore_cmd: ShellCommands::same("mvn dependency:resolve"),
};

static KOTLIN_TEST: TfDescriptor = TfDescriptor {
    name: "kotlin-test",
    display_name: "kotlin.test",
    config_files: &["build.gradle.kts", "build.gradle"],
    config_content_match: Some("kotlin.test"),
    package_json_dep: None,
    discovery_cmd: None,
    run_cmd: ShellCommands::same("./gradlew test"),
    run_single_cmd: ShellCommands::same("./gradlew test --tests {file}"),
};

pub static KOTLIN: LanguageDescriptor = LanguageDescriptor {
    id: "kotlin",
    display_name: "Kotlin",
    file_extensions: &[".kt", ".kts"],
    filenames: &[],
    aliases: &["kt"],
    exclude_dirs: &["build", ".gradle", "out"],
    entry_point_files: &["build.gradle.kts", "build.gradle", "settings.gradle.kts", "pom.xml"],
    sdk: Some(SdkDescriptor {
        name: "Kotlin",
        version_command: "kotlin",
        version_args: &["-version"],
        version_file: None,
        version_json_key: None,
        install_url: "https://kotlinlang.org/docs/command-line.html",
    }),
    package_managers: &[GRADLE, MAVEN],
    test_frameworks: &[KOTLIN_TEST],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
