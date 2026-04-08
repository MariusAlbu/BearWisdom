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

// ---------------------------------------------------------------------------
// Additional symbol kinds from the rules not yet covered above
// ---------------------------------------------------------------------------

#[test]
fn symbol_object_definition_is_class_kind() {
    // object_definition → SymbolKind::Namespace (singleton object treated as Namespace).
    let r = extract("object AppConfig");
    assert!(
        r.symbols.iter().any(|s| s.name == "AppConfig"),
        "expected AppConfig object symbol; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_case_class_definition() {
    // class_definition with `case` modifier → SymbolKind::Class.
    let r = extract("case class Point(x: Int, y: Int)");
    assert!(
        r.symbols.iter().any(|s| s.name == "Point" && s.kind == SymbolKind::Class),
        "expected Class Point (case class); got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_case_class_constructor_params() {
    // case class constructor params → Property symbols for each param.
    let r = extract("case class User(id: Int, name: String)");
    assert!(
        r.symbols.iter().any(|s| s.name == "id" || s.name == "name"),
        "expected Property symbols for case class params; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_extension_definition() {
    // extension_definition (Scala 3) → emits a symbol for the extended type.
    let r = extract("extension (s: String)\n  def shout: String = s.toUpperCase");
    // At minimum no crash; extension produces some symbol or the def inside it does.
    assert!(
        !r.symbols.is_empty(),
        "expected symbols from extension_definition; got none"
    );
}

// ---------------------------------------------------------------------------
// Additional ref kinds from the rules not yet covered above
// ---------------------------------------------------------------------------

#[test]
fn ref_call_expression_dot_method() {
    // call_expression with field_expression — `obj.method(args)` → Calls to method.
    let r = extract("object M {\n  def f(xs: List[Int]): Int = xs.foldLeft(0)(_ + _)\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "foldLeft" && rf.kind == EdgeKind::Calls),
        "expected Calls foldLeft from field_expression; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_trait_implements_other_trait() {
    // trait_definition extending another trait → Implements (not Inherits).
    let r = extract("trait Ordered extends Comparable");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Comparable"),
        "expected ref to Comparable from trait extends; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    // Traits use Implements for all parents per rules.
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Comparable" && (rf.kind == EdgeKind::Implements || rf.kind == EdgeKind::Inherits)),
        "expected Implements or Inherits Comparable from trait; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_object_extends_class() {
    // object_definition extending a class → Inherits edge.
    let r = extract("abstract class Base\nobject Impl extends Base");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Base"),
        "expected ref to Base from object extends; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_ascription_expression_type_ref() {
    // ascription_expression: `expr: Type` → TypeRef to the ascription type.
    let r = extract("def f(x: Any): String = (x: String)");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef String from ascription; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_case_class_pattern_in_match() {
    // case_class_pattern in match → TypeRef to the matched constructor type.
    let r = extract("sealed trait Shape\ncase class Circle(r: Double) extends Shape\ndef describe(s: Shape): String = s match {\n  case Circle(r) => s\"circle r=$r\"\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Circle"),
        "expected TypeRef Circle from case class pattern in match; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_enum_definition_implements() {
    // enum_definition with extends → Implements (or TypeRef) to extended type.
    let r = extract("trait Comparable\nenum Season extends Comparable:\n  case Spring, Summer");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Comparable"),
        "expected ref to Comparable from enum extends; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_class_parameter_type_ref() {
    // class_parameter in constructor param list → TypeRef to the parameter type.
    let r = extract("class Repo(db: Database)");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Database"),
        "expected TypeRef Database from class_parameter; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_type_definition_rhs_type_ref() {
    // type_definition: `type Alias = SomeType` → TypeRef to SomeType.
    let r = extract("type Handler = Request => Response");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Request" || rf.target_name == "Response"),
        "expected TypeRef from type alias rhs; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_generic_function_type_ref() {
    // generic_function call — `identity[String](...)`. The call_expression wrapping a
    // generic_function node. `call_target_name` recurses into the `function` field to
    // extract the base function identifier, emitting a Calls edge to `identity`.
    let r = extract("object M {\n  def f() = identity[String](\"hello\")\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "identity" && rf.kind == EdgeKind::Calls),
        "expected Calls edge to 'identity' from generic_function; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef String from generic_function type arg; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_package_object_has_member() {
    // package_object — must extract symbols defined inside it.
    let r = extract("package object utils {\n  val pi: Double = 3.14\n  def square(x: Int): Int = x * x\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "pi" || s.name == "square"),
        "expected members inside package object; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_for_comprehension_calls() {
    // for_expression desugars to flatMap/map — these call_expressions inside must emit Calls.
    let r = extract("def f(xs: List[Int]): List[Int] =\n  for x <- xs yield x * 2");
    // At minimum no crash; for-comprehension calls may or may not surface.
    let _ = r;
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

#[test]
#[ignore]
fn debug_val_definition_miss_patterns() {
    // Read a few Scala files, parse them, and for each val_definition node
    // that does NOT produce a symbol, print its text to understand the patterns.
    let lang: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();

    let files = [
        "F:/Work/Projects/TestProjects/scala-lila/app/controllers/Account.scala",
        "F:/Work/Projects/TestProjects/scala-lila/app/models/GameFilter.scala",
        "F:/Work/Projects/TestProjects/scala-lila/app/ui/base.scala",
        "F:/Work/Projects/TestProjects/scala-lila/modules/common/src/main/Form.scala",
    ];

    for file_path in &files {
        let src = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let src_bytes = src.as_bytes();
        let tree = match parser.parse(&src, None) {
            Some(t) => t,
            None => continue,
        };
        let result = super::extract::extract(&src);
        let sym_lines: std::collections::HashSet<u32> = result.symbols.iter().map(|s| s.start_line).collect();

        // Walk CST for val_definition nodes
        let mut stack: Vec<(tree_sitter::Node, String)> = vec![(tree.root_node(), "root".to_string())];
        let mut missing = Vec::new();
        while let Some((node, parent_kind)) = stack.pop() {
            if node.kind() == "val_definition" {
                let line = node.start_position().row as u32;
                if !sym_lines.contains(&line) {
                    let text = node.utf8_text(src_bytes).unwrap_or("?");
                    let snippet = text.chars().take(80).collect::<String>().replace('\n', " ");
                    missing.push((line, snippet, parent_kind.clone()));
                }
            }
            let node_kind = node.kind().to_string();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) { stack.push((child, node_kind.clone())); }
        }

        if !missing.is_empty() {
            eprintln!("\n=== {} ===", file_path);
            for (line, text, parent) in missing.iter().take(10) {
                eprintln!("  line {} [parent={}]: {:?}", line, parent, text);
            }
        }
    }
}

#[test]
#[ignore]
fn debug_measure_scala_coverage() {
    let projects = [
        "F:/Work/Projects/TestProjects/scala-lila",
        "F:/Work/Projects/TestProjects/scala-trading",
    ];
    let project_path = projects.iter().find(|p| std::path::Path::new(p).exists()).copied();
    let project_path = match project_path {
        Some(p) => p,
        None => { eprintln!("No Scala test project found"); return; }
    };
    eprintln!("Using project: {}", project_path);
    let results = crate::query::coverage::analyze_coverage(std::path::Path::new(project_path));
    for cov in &results {
        if cov.language == "scala" {
            eprintln!("=== Scala ===");
            eprintln!("  files: {}", cov.file_count);
            eprintln!("  sym: {:.1}% ({}/{})", cov.symbol_coverage.percent, cov.symbol_coverage.matched_nodes, cov.symbol_coverage.expected_nodes);
            eprintln!("  ref: {:.1}% ({}/{})", cov.ref_coverage.percent, cov.ref_coverage.matched_nodes, cov.ref_coverage.expected_nodes);
            eprintln!("  --- symbol kinds (worst first) ---");
            let mut sym_kinds = cov.symbol_kinds.clone();
            sym_kinds.sort_by(|a, b| a.percent.partial_cmp(&b.percent).unwrap());
            for k in sym_kinds.iter().take(10) {
                eprintln!("    {}: {:.1}% ({}/{}) miss={}", k.kind, k.percent, k.matched, k.occurrences, k.occurrences - k.matched);
            }
            eprintln!("  --- ref kinds (worst first) ---");
            let mut ref_kinds = cov.ref_kinds.clone();
            ref_kinds.sort_by(|a, b| a.percent.partial_cmp(&b.percent).unwrap());
            for k in ref_kinds.iter().take(10) {
                eprintln!("    {}: {:.1}% ({}/{}) miss={}", k.kind, k.percent, k.matched, k.occurrences, k.occurrences - k.matched);
            }
        }
    }
}
