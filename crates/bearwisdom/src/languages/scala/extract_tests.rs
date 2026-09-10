use super::*;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn package_clause_emits_namespace() {
    let r = extract::extract("package foo.bar");
    assert!(
        r.symbols
            .iter()
            .any(|s| s.name == "bar" && s.kind == SymbolKind::Namespace),
        "expected Namespace 'bar' from package_clause; got {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn brace_less_package_prefixes_top_level_class_qname() {
    let src = "package cats.effect\n\nclass IO";
    let r = extract::extract(src);
    let io = r.symbols.iter().find(|s| s.name == "IO").expect("IO");
    assert_eq!(
        io.qualified_name, "cats.effect.IO",
        "expected qname 'cats.effect.IO'; got {:?}",
        io.qualified_name
    );
    assert_eq!(io.scope_path.as_deref(), Some("cats.effect"));
}

#[test]
fn brace_less_package_prefixes_nested_class_qname() {
    let src = "package cats.effect\n\nclass IO {\n  class Attempt\n}";
    let r = extract::extract(src);
    let attempt = r
        .symbols
        .iter()
        .find(|s| s.name == "Attempt")
        .expect("Attempt");
    assert_eq!(attempt.qualified_name, "cats.effect.IO.Attempt");
}

#[test]
fn brace_less_package_prefixes_top_level_def() {
    let src = "package cats.effect\n\ndef helper(): Int = 42";
    let r = extract::extract(src);
    let helper = r
        .symbols
        .iter()
        .find(|s| s.name == "helper")
        .expect("helper");
    assert_eq!(helper.qualified_name, "cats.effect.helper");
}

#[test]
fn brace_less_package_prefixes_case_class_params() {
    let src = "package p\n\ncase class Foo(x: Int, y: Int)";
    let r = extract::extract(src);
    let x = r.symbols.iter().find(|s| s.name == "x").expect("x");
    assert_eq!(
        x.qualified_name, "p.Foo.x",
        "case-class param qname should pick up the package prefix"
    );
}

#[test]
fn brace_form_package_prefixes_inner_class() {
    let src = "package foo.bar {\n  class X\n}";
    let r = extract::extract(src);
    let x = r.symbols.iter().find(|s| s.name == "X").expect("X");
    assert_eq!(x.qualified_name, "foo.bar.X");
}

#[test]
fn no_package_leaves_qname_unprefixed() {
    let src = "class Standalone";
    let r = extract::extract(src);
    let s = r
        .symbols
        .iter()
        .find(|s| s.name == "Standalone")
        .expect("Standalone");
    assert_eq!(s.qualified_name, "Standalone");
}

#[test]
fn direct_tuple_val_emits_each_identifier_as_a_binding_symbol() {
    let source = "object O { def f = { val (key, inputs) = make() } }";
    let result = extract::extract(source);
    let tuple_bindings: Vec<_> = result
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Property)
        .filter(|symbol| matches!(symbol.name.as_str(), "key" | "inputs"))
        .collect();

    assert_eq!(tuple_bindings.len(), 2, "{:?}", result.symbols);
    assert!(
        tuple_bindings.iter().any(|symbol| {
            symbol.name == "key" && symbol.byte_offset == source.find("key").unwrap() as u32
        }),
        "{tuple_bindings:?}"
    );
    assert!(
        tuple_bindings.iter().any(|symbol| {
            symbol.name == "inputs" && symbol.byte_offset == source.find("inputs").unwrap() as u32
        }),
        "{tuple_bindings:?}"
    );
}

#[test]
fn direct_tuple_case_patterns_emit_distinct_variables_with_function_parent() {
    let source = r#"object O {
  def decode(value: (Int, Int)) = value match {
    case (left, right) => left + right
    case (left, right) => right + left
  }
}"#;
    let result = extract::extract(source);
    let function_index = result
        .symbols
        .iter()
        .position(|symbol| symbol.name == "decode" && symbol.kind == SymbolKind::Method)
        .expect("decode method");
    let bindings: Vec<_> = result
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Variable)
        .filter(|symbol| matches!(symbol.name.as_str(), "left" | "right"))
        .collect();

    assert_eq!(bindings.len(), 4, "{:?}", result.symbols);
    assert_eq!(
        bindings
            .iter()
            .filter(|symbol| symbol.name == "left")
            .count(),
        2,
        "same-named sibling arms must remain distinct"
    );
    let first_case = source.find("case (left, right)").unwrap();
    let second_case = source.rfind("case (left, right)").unwrap();
    for (name, line, column, offset) in [
        ("left", 2, 10, first_case + "case (".len()),
        ("right", 2, 16, first_case + "case (left, ".len()),
        ("left", 3, 10, second_case + "case (".len()),
        ("right", 3, 16, second_case + "case (left, ".len()),
    ] {
        assert!(
            bindings.iter().any(|binding| {
                binding.name == name
                    && binding.start_line == line
                    && binding.end_line == line
                    && binding.start_col == column
                    && binding.end_col == column + name.len() as u32
                    && binding.byte_offset == offset as u32
                    && binding.parent_index == Some(function_index)
            }),
            "missing exact {name} binding at {line}:{column}; bindings={bindings:?}"
        );
    }
}

#[test]
fn direct_tuple_case_patterns_use_initializer_or_top_level_parent() {
    let source = r#"class C {
  val classResult = value match { case (classLeft, classRight) => 0 }
}
object O {
  val objectResult = value match { case (objectLeft, objectRight) => 0 }
}
value match { case (topLeft, topRight) => 0 }
"#;
    let result = extract::extract(source);
    let class_result = result
        .symbols
        .iter()
        .position(|symbol| symbol.name == "classResult" && symbol.kind == SymbolKind::Property)
        .expect("class initializer property");
    let object_result = result
        .symbols
        .iter()
        .position(|symbol| symbol.name == "objectResult" && symbol.kind == SymbolKind::Property)
        .expect("object initializer property");

    for (name, parent) in [
        ("classLeft", Some(class_result)),
        ("classRight", Some(class_result)),
        ("objectLeft", Some(object_result)),
        ("objectRight", Some(object_result)),
        ("topLeft", None),
        ("topRight", None),
    ] {
        assert!(
            result.symbols.iter().any(|symbol| {
                symbol.kind == SymbolKind::Variable
                    && symbol.name == name
                    && symbol.parent_index == parent
            }),
            "missing {name} with parent {parent:?}; symbols={:?}",
            result.symbols
        );
    }
}

#[test]
fn non_flat_case_patterns_do_not_emit_case_binding_symbols() {
    let cases = [
        ("nested tuple", "case ((nested, pair), outer) => 0"),
        ("typed tuple member", "case (typed: Int, other) => 0"),
        ("extractor", "case Pair(extracted, other) => 0"),
        ("wildcard", "case (_, wildcard) => 0"),
        (
            "named/default tuple member",
            "case (first = one, second = two) => 0",
        ),
        ("default arm", "case _ => 0"),
        ("rest pattern", "case (head, tail*) => 0"),
    ];

    for (label, clause) in cases {
        let source = format!("object O {{ def decode(value: Any) = value match {{ {clause} }} }}");
        let result = extract::extract(&source);
        assert!(
            result
                .symbols
                .iter()
                .all(|symbol| symbol.kind != SymbolKind::Variable),
            "{label} must not emit supported case bindings: {:?}",
            result.symbols
        );
    }
}

#[test]
fn full_enum_case_emits_enum_member() {
    let r = extract::extract("enum Planet:\n  case Earth(mass: Double, radius: Double)");
    assert!(
        r.symbols
            .iter()
            .any(|s| s.name == "Earth" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Earth' from full_enum_case; got {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn simple_enum_case_emits_enum_member() {
    let r = extract::extract("enum Color:\n  case Red, Green, Blue");
    assert!(
        r.symbols
            .iter()
            .any(|s| s.name == "Red" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Red' from simple_enum_case; got {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn extracts_class_and_method() {
    let src = r#"
class Animal(val name: String) {
  def speak(): String = "..."
}
"#;
    let r = extract::extract(src);
    let cls = r
        .symbols
        .iter()
        .find(|s| s.name == "Animal")
        .expect("Animal");
    assert_eq!(cls.kind, SymbolKind::Class);

    let method = r.symbols.iter().find(|s| s.name == "speak").expect("speak");
    assert_eq!(method.kind, SymbolKind::Method);
}

#[test]
fn extracts_trait_and_object() {
    let src = r#"
trait Drawable {
  def draw(): Unit
}

object App {
  def main(args: Array[String]): Unit = {}
}
"#;
    let r = extract::extract(src);
    let tr = r
        .symbols
        .iter()
        .find(|s| s.name == "Drawable")
        .expect("Drawable");
    assert_eq!(tr.kind, SymbolKind::Interface);

    let obj = r.symbols.iter().find(|s| s.name == "App").expect("App");
    assert_eq!(obj.kind, SymbolKind::Namespace);
}

#[test]
fn type_definition_extracted_as_type_alias() {
    let src = r#"
object Aliases {
  type StringMap = Map[String, Int]
  type Callback = Int => Unit
}
"#;
    let r = extract::extract(src);
    assert!(
        r.symbols
            .iter()
            .any(|s| s.name == "StringMap" && s.kind == SymbolKind::TypeAlias),
        "StringMap TypeAlias not found; symbols: {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
    assert!(r
        .symbols
        .iter()
        .any(|s| s.name == "Callback" && s.kind == SymbolKind::TypeAlias));
}

#[test]
fn infix_expression_emits_calls_edge() {
    let src = r#"
def process(a: Int, b: Int): Int = a + b
"#;
    let r = extract::extract(src);
    assert!(
        r.refs
            .iter()
            .any(|rf| rf.target_name == "+" && rf.kind == EdgeKind::Calls),
        "Calls edge for '+' not found; refs: {:?}",
        r.refs
            .iter()
            .map(|rf| (&rf.target_name, rf.kind))
            .collect::<Vec<_>>()
    );
}

#[test]
fn import_produces_import_ref() {
    let src = "import scala.collection.mutable.ListBuffer\n";
    let r = extract::extract(src);
    let imports: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .collect();
    assert!(!imports.is_empty(), "expected import ref");
    let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
    assert!(
        targets.contains(&"ListBuffer"),
        "missing ListBuffer: {targets:?}"
    );
}

#[test]
fn direct_underscore_import_emits_canonical_wildcard_metadata() {
    let r = extract::extract("import pkg.api._\n");
    let wildcard: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports && r.target_name == "*")
        .collect();
    assert_eq!(
        wildcard.len(),
        1,
        "expected one wildcard ref: {:#?}",
        r.refs
    );
    assert_eq!(wildcard[0].module.as_deref(), Some("pkg.api"));
    assert!(!wildcard[0].is_reexport);
}

#[test]
fn noncanonical_scala_import_forms_do_not_emit_wildcard_metadata() {
    for source in [
        "import pkg.api.*",                // Scala 3 wildcard
        "import pkg.api.{Thing, Other}",   // selector group
        "import pkg.api.{Thing as Alias}", // rename
        "import pkg.api.{Thing => _}",     // exclusion
        "import pkg.api.given",            // given-only import
    ] {
        let r = extract::extract(source);
        assert!(
            !r.refs
                .iter()
                .any(|r| r.kind == EdgeKind::Imports && r.target_name == "*"),
            "{source:?} must not become a wildcard import: {:#?}",
            r.refs
        );
    }
}
