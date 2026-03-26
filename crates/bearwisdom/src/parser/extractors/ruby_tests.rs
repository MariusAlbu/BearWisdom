    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    #[test]
    fn extracts_class_and_method() {
        let source = r#"
class Animal
  def initialize(name)
    @name = name
  end

  def speak
    "..."
  end
end
"#;
        let r = extract(source);
        assert!(!r.has_errors);
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal class");
        assert_eq!(cls.kind, SymbolKind::Class);

        let init = r.symbols.iter().find(|s| s.name == "initialize").expect("initialize");
        assert_eq!(init.kind, SymbolKind::Constructor);
        assert_eq!(init.qualified_name, "Animal::initialize");

        let speak = r.symbols.iter().find(|s| s.name == "speak").expect("speak");
        assert_eq!(speak.kind, SymbolKind::Method);
    }

    #[test]
    fn extracts_module() {
        let source = "module Greetable\n  def greet\n    puts 'hi'\n  end\nend\n";
        let r = extract(source);
        let m = r.symbols.iter().find(|s| s.name == "Greetable").expect("Greetable");
        assert_eq!(m.kind, SymbolKind::Interface);
    }

    #[test]
    fn require_produces_import_ref() {
        let source = "require 'net/http'\n";
        let r = extract(source);
        let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).expect("import ref");
        assert_eq!(imp.target_name, "http");
        assert_eq!(imp.module.as_deref(), Some("net"));
    }

    #[test]
    fn require_relative_produces_import_ref() {
        let source = "require_relative '../models/user'\n";
        let r = extract(source);
        let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).expect("import ref");
        assert_eq!(imp.target_name, "user");
    }

    #[test]
    fn inheritance_produces_inherits_edge() {
        let source = "class Dog < Animal\nend\n";
        let r = extract(source);
        let inh = r.refs.iter().find(|r| r.kind == EdgeKind::Inherits).expect("inherits ref");
        assert_eq!(inh.target_name, "Animal");
    }

    #[test]
    fn attr_accessor_produces_property_symbols() {
        let source = r#"
class Person
  attr_accessor :name, :age
end
"#;
        let r = extract(source);
        let props: Vec<_> = r.symbols.iter().filter(|s| s.kind == SymbolKind::Property).collect();
        let names: Vec<&str> = props.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"name"), "name missing: {names:?}");
        assert!(names.contains(&"age"), "age missing: {names:?}");
    }

    #[test]
    fn underscore_method_is_private() {
        let source = "def _helper\nend\n";
        let r = extract(source);
        let sym = r.symbols.iter().find(|s| s.name == "_helper").expect("_helper");
        assert_eq!(sym.visibility, Some(Visibility::Private));
    }

    #[test]
    fn class_new_produces_instantiates_edge() {
        let source = r#"
class Foo
  def build
    Bar.new
  end
end
"#;
        let r = extract(source);
        let inst = r.refs.iter().find(|r| r.kind == EdgeKind::Instantiates);
        assert!(inst.is_some(), "Expected Instantiates edge for Bar.new");
        assert_eq!(inst.unwrap().target_name, "Bar");
    }

    #[test]
    fn handles_parse_errors_gracefully() {
        let source = "class Broken\ndef bad(\nend\n{{{";
        let result = std::panic::catch_unwind(|| extract(source));
        assert!(result.is_ok(), "extractor panicked on malformed input");
    }
