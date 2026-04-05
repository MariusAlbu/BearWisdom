fn main() {
    let src_dir = std::path::Path::new("src");

    let mut c = cc::Build::new();
    c.std("c11").include(src_dir);

    // On MSVC, parser.c fails with:
    //   - TSFieldMapSlice undeclared (grammar uses old type name)
    //   - REDUCE(.symbol=X, .child_count=N) named macro args (C99/MSVC incompatible)
    // parser_expanded.c fixes both issues. On GCC/Clang use the original.
    #[cfg(target_env = "msvc")]
    {
        c.flag("-utf-8");
        c.file(src_dir.join("parser_expanded.c"));
    }
    #[cfg(not(target_env = "msvc"))]
    c.file(src_dir.join("parser.c"));

    c.file(src_dir.join("scanner.c"));

    // GCC/Clang: suppress common warnings from generated parser code.
    c.flag_if_supported("-Wno-unused-parameter");
    c.flag_if_supported("-Wno-unused-but-set-variable");
    c.flag_if_supported("-Wno-trigraphs");

    c.compile("tree-sitter-scss");
}
