fn main() {
    let lang: tree_sitter::Language = tree_sitter_gleam::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();

    let src = "pub type Color { Red }";
    let tree = parser.parse(src, None).unwrap();
    let src_bytes = src.as_bytes();
    
    // Find type_name
    fn scan(node: tree_sitter::Node, src: &[u8], depth: usize) {
        let indent = "  ".repeat(depth);
        let t = std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("?");
        println!("{}{} text={:?}", indent, node.kind(), &t[..t.len().min(30)]);
        if node.kind() == "type_name" {
            let mut i = 0u32;
            let mut c = node.walk();
            for ch in node.children(&mut c) {
                println!("  {} type_name child [{}]: kind={} field={:?} text={:?}", indent, i, ch.kind(), 
                    node.field_name_for_child(i),
                    std::str::from_utf8(&src[ch.start_byte()..ch.end_byte()]).unwrap_or("?"));
                i += 1;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            scan(child, src, depth + 1);
        }
    }
    scan(tree.root_node(), src_bytes, 0);
}
