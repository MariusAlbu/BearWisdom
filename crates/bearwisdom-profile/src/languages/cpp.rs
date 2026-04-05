use crate::types::*;

static CTEST: TfDescriptor = TfDescriptor {
    name: "ctest",
    display_name: "CTest",
    config_files: &["CMakeLists.txt"],
    config_content_match: Some("enable_testing"),
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("ctest --test-dir build -N")),
    run_cmd: ShellCommands::same("ctest --test-dir build"),
    run_single_cmd: ShellCommands::same("ctest --test-dir build -R {file}"),
};

static GTEST: TfDescriptor = TfDescriptor {
    name: "gtest",
    display_name: "Google Test",
    config_files: &["CMakeLists.txt"],
    config_content_match: Some("GTest"),
    package_json_dep: None,
    discovery_cmd: None,
    run_cmd: ShellCommands::same("ctest --test-dir build"),
    run_single_cmd: ShellCommands::same("./build/{file} --gtest_filter=*"),
};

static CMAKE_BUILD: RestoreStep = RestoreStep {
    id: "cmake-configure-cpp",
    title: "Configure CMake build",
    description: "Run CMake to generate build files for C++ project.",
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

pub static CPP: LanguageDescriptor = LanguageDescriptor {
    id: "cpp",
    display_name: "C++",
    file_extensions: &[".cpp", ".cc", ".cxx", ".c++", ".hpp", ".hxx", ".h++", ".hh"],
    filenames: &[],
    aliases: &["c++", "cxx"],
    exclude_dirs: &[
        "build",
        "cmake-build-debug",
        "cmake-build-release",
        ".cmake",
        // Amalgamated single-header distributions (e.g. entt, nlohmann/json,
        // stb, sqlite, etc.).  Indexing these inflates symbol counts with
        // duplicate definitions already covered by the canonical source tree.
        "single_include",
        "single_header",
        "amalgam",
        "amalgamation",
    ],
    entry_point_files: &["CMakeLists.txt", "Makefile", "conanfile.txt", "vcpkg.json"],
    sdk: Some(SdkDescriptor {
        name: "GCC / Clang (C++)",
        version_command: "g++",
        version_args: &["--version"],
        version_file: None,
        version_json_key: None,
        install_url: "https://gcc.gnu.org",
    }),
    package_managers: &[],
    test_frameworks: &[CTEST, GTEST],
    restore_steps: &[CMAKE_BUILD],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
