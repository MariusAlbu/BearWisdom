use crate::types::*;

pub static HASKELL: LanguageDescriptor = LanguageDescriptor {
    id: "haskell",
    display_name: "Haskell",
    file_extensions: &[".hs", ".lhs"],
    filenames: &[],
    aliases: &["hs"],
    exclude_dirs: &[".stack-work", "dist-newstyle", ".cabal-sandbox"],
    entry_point_files: &["stack.yaml", "cabal.project"],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("--"),
    block_comment: Some(("{-", "-}")),
};
