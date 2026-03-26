    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    #[test]
    fn extracts_class_and_method() {
        let source = r#"<?php
class Animal {
    public function __construct(string $name) {
        $this->name = $name;
    }

    public function speak(): string {
        return "...";
    }
}
"#;
        let r = extract(source);
        assert!(!r.has_errors, "unexpected parse errors");

        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal");
        assert_eq!(cls.kind, SymbolKind::Class);

        let ctor = r.symbols.iter().find(|s| s.name == "__construct").expect("__construct");
        assert_eq!(ctor.kind, SymbolKind::Constructor);

        let speak = r.symbols.iter().find(|s| s.name == "speak").expect("speak");
        assert_eq!(speak.kind, SymbolKind::Method);
        assert_eq!(speak.qualified_name, "Animal.speak");
    }

    #[test]
    fn extracts_interface() {
        let source = "<?php\ninterface Drawable {\n    public function draw(): void;\n}\n";
        let r = extract(source);
        let iface = r.symbols.iter().find(|s| s.name == "Drawable").expect("Drawable");
        assert_eq!(iface.kind, SymbolKind::Interface);
    }

    #[test]
    fn use_statement_produces_import_ref() {
        let source = "<?php\nuse App\\Models\\User;\n";
        let r = extract(source);
        let imp = r.refs.iter().find(|r| r.kind == EdgeKind::Imports).expect("import ref");
        assert_eq!(imp.target_name, "User");
        assert_eq!(imp.module.as_deref(), Some("App\\Models"));
    }

    #[test]
    fn extends_produces_inherits_edge() {
        let source = "<?php\nclass Dog extends Animal {}\n";
        let r = extract(source);
        let inh = r.refs.iter().find(|r| r.kind == EdgeKind::Inherits).expect("inherits edge");
        assert_eq!(inh.target_name, "Animal");
    }

    #[test]
    fn implements_produces_implements_edge() {
        let source = "<?php\nclass Cat extends Animal implements Drawable, Serializable {}\n";
        let r = extract(source);
        let impl_refs: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Implements).collect();
        let names: Vec<&str> = impl_refs.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"Drawable"), "missing Drawable: {names:?}");
        assert!(names.contains(&"Serializable"), "missing Serializable: {names:?}");
    }

    #[test]
    fn property_visibility_extracted() {
        let source = r#"<?php
class Foo {
    public string $bar;
    private int $baz;
}
"#;
        let r = extract(source);
        let bar = r.symbols.iter().find(|s| s.name == "bar").expect("bar");
        assert_eq!(bar.kind, SymbolKind::Property);
        assert_eq!(bar.visibility, Some(Visibility::Public));

        let baz = r.symbols.iter().find(|s| s.name == "baz").expect("baz");
        assert_eq!(baz.visibility, Some(Visibility::Private));
    }

    #[test]
    fn enum_and_cases_extracted() {
        let source = r#"<?php
enum Status {
    case Active;
    case Inactive;
}
"#;
        let r = extract(source);
        let en = r.symbols.iter().find(|s| s.name == "Status").expect("Status");
        assert_eq!(en.kind, SymbolKind::Enum);

        let active = r.symbols.iter().find(|s| s.name == "Active").expect("Active");
        assert_eq!(active.kind, SymbolKind::EnumMember);
    }

    #[test]
    fn method_call_produces_calls_edge() {
        let source = r#"<?php
class Foo {
    public function run(): void {
        $this->helper();
    }
    private function helper(): void {}
}
"#;
        let r = extract(source);
        let call = r.refs.iter().find(|r| r.kind == EdgeKind::Calls && r.target_name == "helper");
        assert!(call.is_some(), "Expected Calls edge to 'helper'");
    }

    #[test]
    fn new_produces_instantiates_edge() {
        let source = r#"<?php
function build() {
    return new Foo();
}
"#;
        let r = extract(source);
        let inst = r.refs.iter().find(|r| r.kind == EdgeKind::Instantiates);
        assert!(inst.is_some(), "Expected Instantiates edge");
        assert_eq!(inst.unwrap().target_name, "Foo");
    }

    #[test]
    fn handles_parse_errors_gracefully() {
        let source = "<?php\nclass Broken {\n  function bad(\n}\n{{{";
        let result = std::panic::catch_unwind(|| extract(source));
        assert!(result.is_ok(), "extractor panicked on malformed input");
    }
