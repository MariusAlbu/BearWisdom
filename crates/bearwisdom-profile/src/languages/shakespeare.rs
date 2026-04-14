//! Yesod "Shakespearean" template formats.

use crate::types::*;

pub static HAMLET: LanguageDescriptor = LanguageDescriptor {
    id: "hamlet", display_name: "Hamlet (Yesod)",
    file_extensions: &[".hamlet", ".shamlet", ".whamlet"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: None,
};

pub static CASSIUS: LanguageDescriptor = LanguageDescriptor {
    id: "cassius", display_name: "Cassius (Yesod)",
    file_extensions: &[".cassius"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: None,
};

pub static LUCIUS: LanguageDescriptor = LanguageDescriptor {
    id: "lucius", display_name: "Lucius (Yesod)",
    file_extensions: &[".lucius"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: None,
};

pub static JULIUS: LanguageDescriptor = LanguageDescriptor {
    id: "julius", display_name: "Julius (Yesod)",
    file_extensions: &[".julius"],
    filenames: &[], aliases: &[], exclude_dirs: &[],
    entry_point_files: &[], sdk: None, package_managers: &[],
    test_frameworks: &[], restore_steps: &[],
    line_comment: None, block_comment: None,
};
