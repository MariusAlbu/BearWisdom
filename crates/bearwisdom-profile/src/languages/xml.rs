use crate::types::*;

pub static XML: LanguageDescriptor = LanguageDescriptor {
    id: "xml",
    display_name: "XML",
    file_extensions: &[".xml", ".xsl", ".xslt", ".xsd", ".svg", ".wsdl"],
    filenames: &[],
    aliases: &[],
    exclude_dirs: &[],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: None,
    block_comment: Some(("<!--", "-->")),
};
