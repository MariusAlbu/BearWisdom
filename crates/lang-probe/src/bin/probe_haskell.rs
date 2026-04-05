fn dump(node: tree_sitter::Node, src: &[u8], depth: usize) {
    if depth > 8 { return; }
    let indent = "  ".repeat(depth);
    let text = if node.child_count() == 0 {
        let t = std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("?");
        format!(" = {:?}", &t[..t.len().min(40)])
    } else { String::new() };
    let mut extra = String::new();
    for f in &["name", "variable", "function", "id"] {
        if let Some(n) = node.child_by_field_name(f) {
            let t = std::str::from_utf8(&src[n.start_byte()..n.end_byte()]).unwrap_or("?");
            extra.push_str(&format!(" [{}={}]", f, &t[..t.len().min(20)]));
        }
    }
    println!("{}{}{}{}", indent, node.kind(), text, extra);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        dump(child, src, depth + 1);
    }
}

fn main() {
    let lang: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();

    let src = r#"module Foo where

import Data.Map.Strict (Map)
import qualified Data.Map.Strict as Map

main :: IO ()
main = do
  result <- lookup key myMap
  let val = process result
  return val
  where
    myMap = Map.fromList [("a", 1)]

process x = map toUpper x

helper = fmap negate . filter even
"#;

    let tree = parser.parse(src, None).unwrap();
    println!("has_error: {}", tree.root_node().has_error());
    dump(tree.root_node(), src.as_bytes(), 0);
}
