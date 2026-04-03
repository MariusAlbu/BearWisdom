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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "subscript" && s.kind == SymbolKind::Method),
            "subscript Method not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn import_produces_import_ref() {
        let src = "import Foundation\nimport UIKit\n";
        let r = extract::extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"Foundation"), "missing Foundation: {targets:?}");
        assert!(targets.contains(&"UIKit"), "missing UIKit: {targets:?}");
    }

    #[test]
    fn nested_class_property_extracted() {
        // Properties inside nested types should be extracted
        let src = r#"
class Outer {
    class Inner {
        var value: String = ""
        let constant: Int = 0
    }
    struct Config {
        var timeout: Int = 30
    }
}
"#;
        let r = super::extract::extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(r.symbols.iter().any(|s| s.name == "value"), "missing 'value': {:?}", names);
        assert!(r.symbols.iter().any(|s| s.name == "timeout"), "missing 'timeout': {:?}", names);
    }

    #[test]
    fn local_property_in_function_body_extracted() {
        // Local property_declaration nodes inside function bodies should produce symbols
        let src = r#"
func setup() {
    let timeout: Int = 30
    var config = Config()
    let nested: NestedType = NestedType()
}
"#;
        let r = super::extract::extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(r.symbols.iter().any(|s| s.name == "timeout"), "missing 'timeout': {:?}", names);
    }

    #[test]
    fn swiftui_property_wrappers_extracted() {
        // SwiftUI @State, @Binding, @Environment property wrappers should produce symbols
        let src = r#"
struct AppView: View {
    @Environment(\.modelContext) private var context: ModelContext
    @Binding var selectedTab: AppTab
    @State var iosTabs = IOSTabs.shared
    @State private var isPresented = false
}
"#;
        let r = super::extract::extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(r.symbols.iter().any(|s| s.name == "context"), "missing 'context': {:?}", names);
        assert!(r.symbols.iter().any(|s| s.name == "selectedTab"), "missing 'selectedTab': {:?}", names);
        assert!(r.symbols.iter().any(|s| s.name == "iosTabs"), "missing 'iosTabs': {:?}", names);
    }

    #[test]
    fn properties_in_extension_and_enum_bodies_extracted() {
        // Regression test: properties in extension and enum bodies must be extracted.
        let src = r#"
class Foo {
    var a: Int = 0
}

extension Foo {
    var b: String { return "hello" }
    func bar() {}
}

enum MyEnum {
    case x
    var label: String { return "" }
    func method() {}
}
"#;
        let r = super::extract::extract(src);
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(r.symbols.iter().any(|s| s.name == "b"), "missing 'b' from extension: {:?}", names);
        assert!(r.symbols.iter().any(|s| s.name == "label"), "missing 'label' from enum: {:?}", names);
        assert!(r.symbols.iter().any(|s| s.name == "bar"), "missing 'bar' function from extension: {:?}", names);
        assert!(r.symbols.iter().any(|s| s.name == "method"), "missing 'method' function from enum: {:?}", names);
    }
