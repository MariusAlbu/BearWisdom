#[test]
fn probe_haskell_node_structure() {
    let lang: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    
    let src = "module Test where\n\nfoo :: Int -> Int\nfoo x = x + 1\n\nbar = 42\n\nmain = do\n  putStrLn \"hello\"\n";
    let tree = parser.parse(src, None).unwrap();
    
    fn print_tree(node: tree_sitter::Node, src: &[u8], depth: usize) {
        if depth > 4 { return; }
        let indent = "  ".repeat(depth);
        let text = if node.child_count() == 0 {
            let t = std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("?");
            format!(" = {:?}", &t[..t.len().min(30)])
        } else { String::new() };
        println!("{}{} [{}] ({},{})-({}{}){}", 
            indent, node.kind(), 
            if node.is_named() { "N" } else { "A" },
            node.start_position().row, node.start_position().column,
            node.end_position().row, node.end_position().column,
            text);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            print_tree(child, src, depth + 1);
        }
    }
    
    print_tree(tree.root_node(), src.as_bytes(), 0);
    panic!("probe done - check output above");
}
