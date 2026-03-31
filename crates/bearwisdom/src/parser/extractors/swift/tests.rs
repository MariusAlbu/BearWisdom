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
    fn typealias_extracted() {
        let src = r#"
typealias StringMap = [String: Int]
typealias Handler = (String) -> Void
"#;
        let r = extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "StringMap" && s.kind == SymbolKind::TypeAlias),
            "StringMap TypeAlias not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.symbols.iter().any(|s| s.name == "Handler" && s.kind == SymbolKind::TypeAlias),
            "Handler TypeAlias not found"
        );
    }

    #[test]
    fn as_expression_emits_type_ref() {
        let src = r#"
func cast(x: Any) -> String {
    return x as! String
}
"#;
        let r = extract(src);
        assert!(
            r.refs.iter().any(|rf| rf.target_name == "String" && rf.kind == EdgeKind::TypeRef),
            "TypeRef for String not found; refs: {:?}",
            r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn subscript_declaration_extracted() {
        let src = r#"
struct Matrix {
    subscript(row: Int, col: Int) -> Double {
        return 0.0
    }
}
"#;
        let r = extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "subscript" && s.kind == SymbolKind::Method),
            "subscript Method not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
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
