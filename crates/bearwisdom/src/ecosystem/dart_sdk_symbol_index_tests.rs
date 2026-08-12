use super::*;

#[test]
fn scan_top_level_finds_class_and_function() {
    let src = r#"
class MyClass {}
abstract class Base {}
mixin Mixable {}
enum Color { red, green }
extension FooExt on int {}
void myFunction() {}
int get myGetter => 0;
"#;
    let names = scan_dart_top_level(src);
    assert!(
        names.contains(&"MyClass".to_string()),
        "should find MyClass"
    );
    assert!(names.contains(&"Base".to_string()), "should find Base");
    assert!(
        names.contains(&"Mixable".to_string()),
        "should find Mixable"
    );
    assert!(names.contains(&"Color".to_string()), "should find Color");
}

#[test]
fn build_symbol_index_empty_on_no_roots() {
    let index = build_dart_symbol_index(&[]);
    assert!(index.is_empty());
}
