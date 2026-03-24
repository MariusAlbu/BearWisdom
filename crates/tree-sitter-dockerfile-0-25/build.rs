fn main() {
    let src_dir = std::path::Path::new("src");

    let mut c = cc::Build::new();
    c.include(src_dir);
    c.file(src_dir.join("parser.c"));
    c.file(src_dir.join("scanner.c"));

    // Suppress warnings from generated C code.
    c.flag_if_supported("-Wno-unused-parameter");
    c.flag_if_supported("-Wno-unused-but-set-variable");
    c.flag_if_supported("-Wno-trigraphs");

    c.compile("tree-sitter-dockerfile");
}
