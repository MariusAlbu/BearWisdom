fn main() {
    let lang: tree_sitter::Language = tree_sitter_groovy::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();

    // Test with traits, interfaces, and annotations - common in real Groovy
    let src = r#"package com.example

import groovy.transform.CompileStatic

@CompileStatic
class ProtobufPlugin implements Plugin<Project> {
    private static final List<String> PREREQ = ['java']

    @Override
    void apply(Project project) {
        doStuff(project)
    }

    private void doStuff(Project project) {
        println("stuff")
    }
}

interface Plugin<T> {
    void apply(T target)
}

trait Loggable {
    void log(String msg) {
        println(msg)
    }
}
"#;

    fn dump(node: tree_sitter::Node, src: &[u8], depth: usize) {
        if depth > 6 { return; }
        let indent = "  ".repeat(depth);
        let text = if node.child_count() == 0 {
            let t = std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("?");
            format!(" = {:?}", &t[..t.len().min(40)])
        } else { String::new() };
        // Show named field for name/body
        let mut extra = String::new();
        if let Some(n) = node.child_by_field_name("name") {
            let t = std::str::from_utf8(&src[n.start_byte()..n.end_byte()]).unwrap_or("?");
            extra.push_str(&format!(" [name={:?}]", &t[..t.len().min(30)]));
        }
        if let Some(n) = node.child_by_field_name("body") {
            extra.push_str(&format!(" [body=<{}>]", n.kind()));
        }
        println!("{}{}{}{}", indent, node.kind(), text, extra);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            dump(child, src, depth + 1);
        }
    }
    
    let tree = parser.parse(src, None).unwrap();
    println!("has_error: {}", tree.root_node().has_error());
    dump(tree.root_node(), src.as_bytes(), 0);
}
