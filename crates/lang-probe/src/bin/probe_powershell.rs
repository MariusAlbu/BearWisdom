fn dump(node: tree_sitter::Node, src: &[u8], depth: usize) {
    if depth > 10 { return; }
    let indent = "  ".repeat(depth);
    let text = if node.child_count() == 0 {
        let t = std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("?");
        format!(" = {:?}", &t[..t.len().min(40)])
    } else { String::new() };
    println!("{}{}{}", indent, node.kind(), text);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dump(child, src, depth + 1);
    }
}

fn main() {
    let lang: tree_sitter::Language = tree_sitter_powershell::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();

    // Function with param block (common pattern)
    let src = r#"function Get-Data {
    param(
        [string]$Path,
        [int]$Count = 10
    )
    Get-ChildItem -Path $Path
    Write-Host "Found items"
    Invoke-Rest
}
"#;

    let tree = parser.parse(src, None).unwrap();
    println!("has_error: {}", tree.root_node().has_error());
    dump(tree.root_node(), src.as_bytes(), 0);
}
