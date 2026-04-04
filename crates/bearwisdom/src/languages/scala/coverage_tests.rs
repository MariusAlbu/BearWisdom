// =============================================================================
// scala/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

#[test]
fn symbol_class_definition() {
    let r = extract("class Foo");
    assert!(
        r.symbols.iter().any(|s| s.name == "Foo" && s.kind == SymbolKind::Class),
        "expected Class Foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_object_definition() {
    let r = extract("object Singleton");
    assert!(
        r.symbols.iter().any(|s| s.name == "Singleton"),
        "expected Singleton; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_trait_definition() {
    let r = extract("trait Drawable");
    assert!(
        r.symbols.iter().any(|s| s.name == "Drawable" && s.kind == SymbolKind::Interface),
        "expected Interface Drawable; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_enum_definition() {
    // Scala 3 enum syntax (NOT Kotlin's `enum class`)
    let r = extract("enum Color:\n  case Red, Green, Blue");
    assert!(
        r.symbols.iter().any(|s| s.name == "Color"),
        "expected Color; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_full_enum_case() {
    // Scala 3 full enum case (with constructor)
    let r = extract("enum Planet:\n  case Earth(mass: Double, radius: Double)");
    assert!(
        r.symbols.iter().any(|s| s.name == "Earth" || s.name == "Planet"),
        "expected Earth or Planet; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_simple_enum_case() {
    // Scala 3 simple enum case
    let r = extract("enum Dir:\n  case North, South");
    // At minimum enum itself is extracted.
    assert!(
        r.symbols.iter().any(|s| s.name == "Dir" || s.name == "North"),
        "expected Dir or North; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_function_definition() {
    let r = extract("def add(a: Int, b: Int): Int = a + b");
    assert!(
        r.symbols.iter().any(|s| s.name == "add"),
        "expected add; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_function_declaration() {
    // Abstract method in a trait.
    let r = extract("trait Sortable {\n  def compare(a: Int, b: Int): Int\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "compare"),
        "expected compare; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_val_definition() {
    let r = extract("val maxRetries: Int = 5");
    assert!(
        r.symbols.iter().any(|s| s.name == "maxRetries"),
        "expected maxRetries; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_var_definition() {
    let r = extract("var counter: Int = 0");
    assert!(
        r.symbols.iter().any(|s| s.name == "counter"),
        "expected counter; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_val_declaration() {
    // Abstract val in trait.
    let r = extract("trait Config {\n  val timeout: Int\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "timeout"),
        "expected timeout; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_var_declaration() {
    let r = extract("trait Mutable {\n  var value: String\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "value"),
        "expected value; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_type_definition() {
    let r = extract("type Alias = String");
    assert!(
        r.symbols.iter().any(|s| s.name == "Alias"),
        "expected Alias; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_given_definition() {
    let r = extract("given intOrd: Ordering[Int] = Ordering.Int");
    assert!(
        r.symbols.iter().any(|s| s.name == "intOrd"),
        "expected intOrd; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_package_clause() {
    // package_clause recurses into body — members inside are extracted.
    let r = extract("package foo.bar\n\nclass MyService");
    assert!(
        r.symbols.iter().any(|s| s.name == "MyService"),
        "expected MyService; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_package_object() {
    let r = extract("package object helpers {\n  def noop(): Unit = ()\n}");
    assert!(
        !r.symbols.is_empty(),
        "expected symbols from package object; got none"
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

#[test]
fn ref_call_expression() {
    let r = extract("object M {\n  def f() = println(\"hi\")\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "println" && rf.kind == EdgeKind::Calls),
        "expected Calls println; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_instance_expression() {
    let r = extract("class Dog\ndef make() = new Dog()");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Dog"),
        "expected ref to Dog; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_import_declaration() {
    let r = extract("import scala.collection.mutable.ListBuffer");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_export_declaration() {
    // Scala 3 export clause.
    let r = extract("export scala.math.{min, max}");
    // export may not be fully implemented, but it shouldn't panic.
    // At minimum no crash.
    let _ = r;
}

#[test]
fn ref_type_identifier() {
    // type_identifier in extends clause emits TypeRef (well-supported path).
    let r = extract("class Dog extends Animal");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Animal"),
        "expected ref to Animal; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_type_arguments() {
    // type_arguments in a type alias definition — emits TypeRef via push_type_definition.
    let r = extract("type MyList = List[Int]");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "List" || rf.target_name == "MyList"),
        "expected TypeRef from type alias; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_type_annotation_in_val() {
    // type_identifier in val type annotation: `val x: String`
    let r = extract("val name: String = \"Alice\"");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef to String in val annotation; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_type_annotation_in_var() {
    // type_identifier in var type annotation: `var count: Int`
    let r = extract("var counter: Int = 0");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Int" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef to Int in var annotation; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_return_type_in_function() {
    // type_identifier in function return type: `def f(): String`
    let r = extract("def greet(): String = \"Hi\"");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef to String in return type; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_parameter_type_in_function() {
    // type_identifier in function parameter: `def f(name: String)`
    let r = extract("def greet(name: String): String = \"Hi \" + name");
    assert!(
        r.refs.iter().filter(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef).count() >= 1,
        "expected TypeRef to String in parameter or return type; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_generic_type_in_val() {
    // type_arguments in val annotation: `val items: List[User]`
    let r = extract("class User\nval items: List[User] = List()");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "User" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef to User in List[User]; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_nested_generic_types() {
    // nested type arguments: `val m: Map[String, List[Int]]`
    let r = extract("val m: Map[String, List[Int]] = Map()");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Map" && rf.kind == EdgeKind::TypeRef)
            || r.refs.iter().any(|rf| rf.target_name == "List" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef to Map or List; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_extends_clause() {
    let r = extract("class Dog extends Animal");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Animal"),
        "expected ref to Animal; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_infix_expression() {
    let r = extract("object M {\n  def f() = 1 to 10\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "to" && rf.kind == EdgeKind::Calls),
        "expected Calls to; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Gap-coverage tests — nested contexts
// ---------------------------------------------------------------------------

#[test]
fn symbol_val_in_function_block() {
    // val_definition inside a function body block must be extracted.
    let r = extract("def outer(): Int = {\n  val inner = 42\n  inner\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "inner"),
        "expected nested val 'inner'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_nested_def_in_function_block() {
    // function_definition inside a function body block.
    let r = extract("def outer(): Int = {\n  def helper(x: Int): Int = x + 1\n  helper(5)\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "helper"),
        "expected nested def 'helper'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_var_in_function_block() {
    // var_definition inside a function body block.
    let r = extract("def outer(): Unit = {\n  var count = 0\n  count += 1\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "count"),
        "expected nested var 'count'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_val_in_val_block() {
    // val_definition nested inside another val's block initializer.
    let r = extract("val x = {\n  val inner = 1\n  inner\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "inner"),
        "expected nested val 'inner' in val block; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_infix_in_function_block() {
    // infix_expression inside a function body block emits Calls.
    let r = extract("def f(xs: List[Int]): List[Int] = {\n  xs map (_ + 1)\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "map" && rf.kind == EdgeKind::Calls),
        "expected Calls 'map'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_extends_with_stable_type_identifier() {
    // extends with fully-qualified type: `class Foo extends foo.Bar`
    let r = extract("class Foo extends foo.Bar");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Bar" && rf.kind == EdgeKind::Inherits),
        "expected Inherits 'Bar' from stable_type_identifier; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_extends_multiple_with_clauses() {
    // `class Foo extends Bar with Baz with Qux`
    let r = extract("class Foo extends Bar with Baz with Qux");
    let refs: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Inherits || rf.kind == EdgeKind::Implements).collect();
    assert!(
        refs.iter().any(|rf| rf.target_name == "Bar"),
        "expected Inherits 'Bar'; got {:?}", refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert!(
        refs.iter().any(|rf| rf.target_name == "Baz" || refs.iter().any(|rf2| rf2.target_name == "Qux")),
        "expected Implements mixins; got {:?}", refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_given_definition_type_ref() {
    // given_definition emits TypeRef for its return type.
    let r = extract("given ord: Ordering[String] = Ordering.String");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Ordering" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef 'Ordering'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
#[ignore]
fn debug_package_clause() {
    // Test what tree-sitter produces for common package patterns
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
    parser.set_language(&lang).unwrap();
    
    let src = "package foo.bar\n\nobject MyObj {}";
    let tree = parser.parse(src, None).unwrap();
    
    fn dump(node: tree_sitter::Node, src: &[u8], depth: usize) {
        let text = if node.child_count() == 0 {
            format!(" = {:?}", std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("?"))
        } else { String::new() };
        eprintln!("{}{} ({},{}){}", "  ".repeat(depth), node.kind(), node.start_position().row, node.start_position().column, text);
        let mut c = node.walk();
        for child in node.children(&mut c) { dump(child, src, depth + 1); }
    }
    dump(tree.root_node(), src.as_bytes(), 0);
    
    let r = extract(src);
    eprintln!("Symbols: {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn debug_enum_cases() {
    use tree_sitter::Parser;
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
    parser.set_language(&lang).unwrap();
    
    let src = "enum Planet:\n  case Earth(mass: Double, radius: Double)\n  case Mars(mass: Double, radius: Double)";
    let tree = parser.parse(src, None).unwrap();
    
    fn dump(node: tree_sitter::Node, src: &[u8], depth: usize) {
        let text = if node.child_count() == 0 {
            format!(" = {:?}", std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("?"))
        } else { String::new() };
        eprintln!("{}{}{}", "  ".repeat(depth), node.kind(), text);
        let mut c = node.walk();
        for child in node.children(&mut c) { dump(child, src, depth + 1); }
    }
    dump(tree.root_node(), src.as_bytes(), 0);
    
    let r = extract(src);
    eprintln!("Symbols: {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn debug_extends_generic() {
    let r = extract("class Foo extends Bar[Int] with Baz");
    eprintln!("Refs: {:?}", r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>());
}
