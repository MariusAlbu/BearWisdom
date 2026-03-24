use crate::types::*;

pub static SQL: LanguageDescriptor = LanguageDescriptor {
    id: "sql",
    display_name: "SQL",
    file_extensions: &[".sql", ".psql", ".ddl", ".dml"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("--"),
    block_comment: Some(("/*", "*/")),
};
