use crate::types::*;

pub static DART: LanguageDescriptor = LanguageDescriptor {
    id: "dart",
    display_name: "Dart",
    file_extensions: &[".dart"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[".dart_tool", "build", ".pub-cache"],
    entry_point_files: &["pubspec.yaml"],
    sdk: Some(SdkDescriptor {
        name: "Dart",
        version_command: "dart",
        version_args: &["--version"],
        version_file: None,
        version_json_key: None,
        install_url: "https://dart.dev/get-dart",
    }),
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
