fn main() {
    let mut build = cc::Build::new();
    build.std("c11");
    for dialect in ["typescript", "tsx"] {
        let source = std::path::Path::new(dialect).join("src");
        build.include(&source);
        for file in ["parser.c", "scanner.c"] {
            let path = source.join(file);
            println!("cargo:rerun-if-changed={}", path.display());
            build.file(path);
        }
        println!(
            "cargo:rerun-if-changed={}",
            source.join("tree_sitter").display()
        );
    }
    println!("cargo:rerun-if-changed=common/scanner.h");
    build.flag_if_supported("-Wno-unused-parameter");
    build.flag_if_supported("-Wno-unused-but-set-variable");
    build.flag_if_supported("-Wno-trigraphs");
    build.compile("tree-sitter-typescript-local");
}
