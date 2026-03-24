use crate::types::*;

static MAVEN: PmDescriptor = PmDescriptor {
    name: "maven",
    lock_file: None,
    deps_dir: None, // ~/.m2 local repo
    install_cmd: ShellCommands::same("mvn install -DskipTests"),
    restore_cmd: ShellCommands::same("mvn dependency:resolve"),
};

static GRADLE: PmDescriptor = PmDescriptor {
    name: "gradle",
    lock_file: None,
    deps_dir: None,
    install_cmd: ShellCommands::same("./gradlew build -x test"),
    restore_cmd: ShellCommands::same("./gradlew dependencies"),
};

static JUNIT: TfDescriptor = TfDescriptor {
    name: "junit",
    display_name: "JUnit",
    config_files: &["pom.xml", "build.gradle", "build.gradle.kts"],
    config_content_match: Some("junit"),
    package_json_dep: None,
    discovery_cmd: None,
    run_cmd: ShellCommands {
        bash: "mvn test",
        powershell: "mvn test",
        cmd: "mvn test",
    },
    run_single_cmd: ShellCommands::same("mvn test -Dtest={file}"),
};

static GRADLE_TEST: TfDescriptor = TfDescriptor {
    name: "gradle-test",
    display_name: "Gradle Test",
    config_files: &["build.gradle", "build.gradle.kts"],
    config_content_match: None,
    package_json_dep: None,
    discovery_cmd: None,
    run_cmd: ShellCommands::same("./gradlew test"),
    run_single_cmd: ShellCommands::same("./gradlew test --tests {file}"),
};

static MVN_INSTALL: RestoreStep = RestoreStep {
    id: "mvn-install",
    title: "Resolve Maven dependencies",
    description: "Run `mvn dependency:resolve` to download all Maven dependencies.",
    trigger: RestoreTrigger::FileMissing,
    watch_path: "pom.xml",
    commands: ShellCommands::same("mvn dependency:resolve"),
    auto_fixable: true,
    critical: false,
};

pub static JAVA: LanguageDescriptor = LanguageDescriptor {
    id: "java",
    display_name: "Java",
    file_extensions: &[".java"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &["build", "target", ".gradle", "out"],
    entry_point_files: &["pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"],
    sdk: Some(SdkDescriptor {
        name: "Java (JDK)",
        version_command: "java",
        version_args: &["-version"],
        version_file: Some(".java-version"),
        version_json_key: None,
        install_url: "https://adoptium.net",
    }),
    package_managers: &[MAVEN, GRADLE],
    test_frameworks: &[JUNIT, GRADLE_TEST],
    restore_steps: &[MVN_INSTALL],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
