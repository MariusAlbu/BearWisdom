use crate::types::*;

pub static SCALA: LanguageDescriptor = LanguageDescriptor {
    id: "scala",
    display_name: "Scala",
    file_extensions: &[".scala", ".sc"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &["target", ".bsp", ".metals", "project/target"],
    entry_point_files: &["build.sbt", "build.sc"],
    sdk: Some(SdkDescriptor {
        name: "Scala",
        version_command: "scala",
        version_args: &["-version"],
        version_file: None,
        version_json_key: None,
        install_url: "https://www.scala-lang.org/download/",
    }),
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
