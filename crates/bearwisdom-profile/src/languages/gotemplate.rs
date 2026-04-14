use crate::types::*;

pub static GOTEMPLATE: LanguageDescriptor = LanguageDescriptor {
    id: "gotemplate",
    display_name: "Go Template",
    file_extensions: &[".tmpl", ".gotmpl", ".gohtml", ".tpl"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("{{/*", "*/}}")),
};

pub static TEMPL: LanguageDescriptor = LanguageDescriptor {
    id: "templ",
    display_name: "Templ",
    file_extensions: &[".templ"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
};
