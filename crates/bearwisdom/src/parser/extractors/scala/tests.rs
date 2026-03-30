    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn extracts_class_and_method() {
        let src = r#"
class Animal(val name: String) {
  def speak(): String = "..."
}
"#;
        let r = extract(src);
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
        let r = extract(src);
        let tr = r.symbols.iter().find(|s| s.name == "Drawable").expect("Drawable");
        assert_eq!(tr.kind, SymbolKind::Interface);

        let obj = r.symbols.iter().find(|s| s.name == "App").expect("App");
        assert_eq!(obj.kind, SymbolKind::Namespace);
    }

    #[test]
    fn import_produces_import_ref() {
        let src = "import scala.collection.mutable.ListBuffer\n";
        let r = extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        assert!(!imports.is_empty(), "expected import ref");
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"ListBuffer"), "missing ListBuffer: {targets:?}");
    }
