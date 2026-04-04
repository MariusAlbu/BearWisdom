use crate::types::*;

pub static CLOJURE: LanguageDescriptor = LanguageDescriptor {
    id: "clojure",
    display_name: "Clojure",
    file_extensions: &[".clj", ".cljs", ".cljc", ".edn"],
    filenames: &[],
    aliases: &["cljs", "clojurescript"],
    exclude_dirs: &[".cljs_node_repl"],
    entry_point_files: &[],
    sdk: None,
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some(";"),
    block_comment: None,
};
