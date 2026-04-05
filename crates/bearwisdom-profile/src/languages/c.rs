use crate::types::*;

static CMAKE_TEST: TfDescriptor = TfDescriptor {
    name: "ctest",
    display_name: "CTest",
    config_files: &["CMakeLists.txt"],
    config_content_match: Some("enable_testing"),
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands {
        bash: "ctest --test-dir build -N",
        powershell: "ctest --test-dir build -N",
        cmd: "ctest --test-dir build -N",
    }),
    run_cmd: ShellCommands {
        bash: "ctest --test-dir build",
        powershell: "ctest --test-dir build",
        cmd: "ctest --test-dir build",
    },
    run_single_cmd: ShellCommands::same("ctest --test-dir build -R {file}"),
};

static CMAKE_BUILD: RestoreStep = RestoreStep {
    id: "cmake-configure",
    title: "Configure CMake build",
    description: "Run CMake to generate build files in the build/ directory.",
    trigger: RestoreTrigger::DirMissing,
    watch_path: "build",
    commands: ShellCommands {
        bash: "cmake -B build -S . && cmake --build build",
        powershell: "cmake -B build -S . ; cmake --build build",
        cmd: "cmake -B build -S . && cmake --build build",
    },
    auto_fixable: true,
    critical: true,
};

pub static C: LanguageDescriptor = LanguageDescriptor {
    id: "c",
    display_name: "C",
    file_extensions: &[".c", ".h"],
    filenames: &["Makefile"],
    aliases: &[],
    exclude_dirs: &[
        "build",
        "cmake-build-debug",
        "cmake-build-release",
        ".cmake",
        "single_include",
        "single_header",
        "amalgam",
        "amalgamation",
    ],
    entry_point_files: &["CMakeLists.txt", "Makefile", "configure.ac", "meson.build"],
    sdk: Some(SdkDescriptor {
        name: "GCC / Clang",
        version_command: "gcc",
        version_args: &["--version"],
        version_file: None,
        version_json_key: None,
        install_url: "https://gcc.gnu.org",
    }),
    package_managers: &[],
    test_frameworks: &[CMAKE_TEST],
    restore_steps: &[CMAKE_BUILD],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
