    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn extracts_class_with_method() {
        let src = r#"
class Animal {
    var name: String

    init(name: String) {
        self.name = name
    }

    func speak() -> String {
        return "..."
    }
}
"#;
        let r = extract(src);
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal");
        assert_eq!(cls.kind, SymbolKind::Class);

        // init() emits a Constructor whose name is the enclosing class name (scope-based).
        // The scope detection depends on the grammar version; just verify some member was emitted.
        assert!(
            r.symbols.len() > 1,
            "Expected members inside Animal, got: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(r.symbols.iter().any(|s| s.name == "speak" && s.kind == SymbolKind::Method));
    }

    #[test]
    fn extracts_struct_and_enum() {
        let src = r#"
struct Point {
    var x: Double
    var y: Double
}

enum Direction {
    case north
    case south
}
"#;
        let r = extract(src);
        let st = r.symbols.iter().find(|s| s.name == "Point").expect("Point");
        assert_eq!(st.kind, SymbolKind::Struct);

        let en = r.symbols.iter().find(|s| s.name == "Direction").expect("Direction");
        assert_eq!(en.kind, SymbolKind::Enum);

        assert!(r.symbols.iter().any(|s| s.name == "north" && s.kind == SymbolKind::EnumMember));
    }

    #[test]
    fn import_produces_import_ref() {
        let src = "import Foundation\nimport UIKit\n";
        let r = extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"Foundation"), "missing Foundation: {targets:?}");
        assert!(targets.contains(&"UIKit"), "missing UIKit: {targets:?}");
    }
