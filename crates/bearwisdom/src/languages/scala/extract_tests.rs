    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn package_clause_emits_namespace() {
        let r = extract::extract("package foo.bar");
        assert!(
            r.symbols.iter().any(|s| s.name == "bar" && s.kind == SymbolKind::Namespace),
            "expected Namespace 'bar' from package_clause; got {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
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
        let helper = r.symbols.iter().find(|s| s.name == "helper").expect("helper");
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
        let s = r.symbols.iter().find(|s| s.name == "Standalone").expect("Standalone");
        assert_eq!(s.qualified_name, "Standalone");
    }

    #[test]
    fn full_enum_case_emits_enum_member() {
        let r = extract::extract("enum Planet:\n  case Earth(mass: Double, radius: Double)");
        assert!(
            r.symbols.iter().any(|s| s.name == "Earth" && s.kind == SymbolKind::EnumMember),
            "expected EnumMember 'Earth' from full_enum_case; got {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn simple_enum_case_emits_enum_member() {
        let r = extract::extract("enum Color:\n  case Red, Green, Blue");
        assert!(
            r.symbols.iter().any(|s| s.name == "Red" && s.kind == SymbolKind::EnumMember),
            "expected EnumMember 'Red' from simple_enum_case; got {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
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
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal");
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
        let tr = r.symbols.iter().find(|s| s.name == "Drawable").expect("Drawable");
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
            r.symbols.iter().any(|s| s.name == "StringMap" && s.kind == SymbolKind::TypeAlias),
            "StringMap TypeAlias not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(r.symbols.iter().any(|s| s.name == "Callback" && s.kind == SymbolKind::TypeAlias));
    }

    #[test]
    fn infix_expression_emits_calls_edge() {
        let src = r#"
def process(a: Int, b: Int): Int = a + b
"#;
        let r = extract::extract(src);
        assert!(
            r.refs.iter().any(|rf| rf.target_name == "+" && rf.kind == EdgeKind::Calls),
            "Calls edge for '+' not found; refs: {:?}",
            r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn import_produces_import_ref() {
        let src = "import scala.collection.mutable.ListBuffer\n";
        let r = extract::extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        assert!(!imports.is_empty(), "expected import ref");
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"ListBuffer"), "missing ListBuffer: {targets:?}");
    }
