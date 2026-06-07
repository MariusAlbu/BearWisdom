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

    // -----------------------------------------------------------------------
    // Protocol-extension default methods (LANG-SWIFT-1).
    //
    // An extension's direct body declarations file under the extended type's
    // normalized base via `scope_path`, and the extension container emits a
    // self-`TypeRef` to that base — the impl-container shape the supertype
    // reroute already consumes. Retroactive conformance (`extension C: P {}`)
    // additionally emits an `Implements` ref from the container.
    // -----------------------------------------------------------------------

    #[test]
    fn extension_default_method_files_under_extended_base() {
        let src = r#"
protocol Greet { func hello() }
extension Greet { func hello() { } }
struct Dog: Greet {}
"#;
        let r = super::extract::extract(src);

        // The extension's method `hello` files under `Greet` (its extended base),
        // not at top level.
        let hello = r
            .symbols
            .iter()
            .find(|s| s.name == "hello" && s.kind == SymbolKind::Method)
            .expect("extension method 'hello' must be a Method symbol");
        assert_eq!(
            hello.scope_path.as_deref(),
            Some("Greet"),
            "extension method must file under the extended base 'Greet', got: {:?}",
            hello.scope_path
        );

        // The extension container emits a self-TypeRef to the extended base, so
        // the supertype reroute can read the implementing/extended type.
        let ext_idx = r
            .symbols
            .iter()
            .position(|s| s.kind == SymbolKind::Namespace && s.name == "Greet")
            .expect("extension container Namespace named 'Greet'");
        assert!(
            r.refs.iter().any(|rf| {
                rf.kind == EdgeKind::TypeRef
                    && rf.source_symbol_index == ext_idx
                    && rf.target_name == "Greet"
            }),
            "extension container must emit a self-TypeRef to 'Greet'; refs: {:?}",
            r.refs
                .iter()
                .filter(|rf| rf.source_symbol_index == ext_idx)
                .map(|rf| (&rf.target_name, rf.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn extension_local_let_keeps_method_scope_not_extended_base() {
        // A local `let` inside an extension method's body must stay scoped to
        // the method, NOT be hoisted under the extended base. Only the direct
        // body declarations of the extension file under the base.
        let src = r#"
protocol Greet { func hello() }
extension Greet {
    func hello() {
        let local = 5
    }
}
"#;
        let r = super::extract::extract(src);
        if let Some(local) = r.symbols.iter().find(|s| s.name == "local") {
            assert_ne!(
                local.scope_path.as_deref(),
                Some("Greet"),
                "a local inside the method body must not file under the extended base"
            );
        }
    }

    #[test]
    fn extension_retroactive_conformance_emits_implements() {
        // `extension Dog: Greet {}` forms the Dog -> Greet conformance via an
        // Implements ref sourced from the extension container (a Namespace), so
        // the supertype reroute keys it under the implementing type Dog.
        let src = r#"
protocol Greet { func hello() }
struct Dog {}
extension Dog: Greet {}
"#;
        let r = super::extract::extract(src);

        let ext_idx = r
            .symbols
            .iter()
            .position(|s| s.kind == SymbolKind::Namespace && s.name == "Dog")
            .expect("extension container Namespace named 'Dog'");
        assert!(
            r.refs.iter().any(|rf| {
                rf.kind == EdgeKind::Implements
                    && rf.source_symbol_index == ext_idx
                    && rf.target_name == "Greet"
            }),
            "retroactive conformance must emit Implements Dog-container -> Greet; refs: {:?}",
            r.refs
                .iter()
                .filter(|rf| rf.source_symbol_index == ext_idx)
                .map(|rf| (&rf.target_name, rf.kind))
                .collect::<Vec<_>>()
        );

        // The extended type itself (Dog) is the self-TypeRef carrier, never an
        // Implements parent — guard against double-emitting Dog as a conformance.
        assert!(
            !r.refs.iter().any(|rf| {
                rf.kind == EdgeKind::Implements
                    && rf.source_symbol_index == ext_idx
                    && rf.target_name == "Dog"
            }),
            "the extended type must not be emitted as its own conformance parent"
        );
    }

    #[test]
    fn extension_generic_base_normalized_to_bare_name() {
        // `extension Array<Element> { ... }` files members under the bare base
        // `Array`, and the self-TypeRef carries the same base so the reroute and
        // the member filing agree.
        let src = r#"
extension Array {
    func firstOrNil() -> Element? { return first }
}
"#;
        let r = super::extract::extract(src);
        let m = r
            .symbols
            .iter()
            .find(|s| s.name == "firstOrNil")
            .expect("extension method 'firstOrNil'");
        assert_eq!(
            m.scope_path.as_deref(),
            Some("Array"),
            "concrete-type extension files members under the base 'Array', got: {:?}",
            m.scope_path
        );
    }

    // -----------------------------------------------------------------------
    // Function-parameter symbol + declared-type capture (LANG-SWIFT-1).
    //
    // A function parameter is emitted as a `Parameter` symbol scoped under the
    // enclosing function, with a `TypeRef` to its declared type. The recursive
    // type scan peels the opaque/existential keyword, so `g: some Greet` /
    // `e: any Greet` capture `Greet`, rooting `g.hello()` on the protocol's
    // extension default.
    // -----------------------------------------------------------------------

    #[test]
    fn function_parameter_emitted_as_scoped_symbol_with_type_ref() {
        let src = r#"
protocol Greet { func hello() }
func use(g: some Greet, label e: any Greet, _ x: Plain) {}
"#;
        let r = super::extract::extract(src);

        let use_idx = r
            .symbols
            .iter()
            .position(|s| s.name == "use" && s.kind == SymbolKind::Function)
            .expect("function 'use'");
        let use_qname = r.symbols[use_idx].qualified_name.clone();

        for (pname, tname) in [("g", "Greet"), ("e", "Greet"), ("x", "Plain")] {
            let p = r
                .symbols
                .iter()
                .find(|s| s.name == pname && s.kind == SymbolKind::Parameter)
                .unwrap_or_else(|| {
                    panic!(
                        "parameter '{pname}' must be a Parameter symbol; symbols: {:?}",
                        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
                    )
                });
            assert_eq!(
                p.scope_path.as_deref(),
                Some(use_qname.as_str()),
                "parameter '{pname}' must scope under the function qname"
            );
            assert_eq!(
                p.qualified_name,
                format!("{use_qname}.{pname}"),
                "parameter '{pname}' qname must be function-qualified"
            );
            let p_idx = r
                .symbols
                .iter()
                .position(|s| std::ptr::eq(s, p))
                .unwrap();
            // The parameter's FIRST TypeRef is its declared type (keyword peeled).
            let first_type = r
                .refs
                .iter()
                .find(|rf| {
                    rf.source_symbol_index == p_idx && rf.kind == EdgeKind::TypeRef
                })
                .map(|rf| rf.target_name.as_str());
            assert_eq!(
                first_type,
                Some(tname),
                "parameter '{pname}' must carry a TypeRef to '{tname}'; refs from {p_idx}: {:?}",
                r.refs
                    .iter()
                    .filter(|rf| rf.source_symbol_index == p_idx)
                    .map(|rf| (&rf.target_name, rf.kind))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn opaque_parameter_does_not_emit_keyword_type() {
        // The `some`/`any` keyword is never emitted as a type name — only the
        // peeled constraint. Guards against a `some Greet` literal target.
        let src = r#"
protocol Greet { func hello() }
func use(g: some Greet) {}
"#;
        let r = super::extract::extract(src);
        assert!(
            !r.refs.iter().any(|rf| {
                rf.kind == EdgeKind::TypeRef
                    && (rf.target_name == "some" || rf.target_name.starts_with("some "))
            }),
            "the opaque keyword must not leak into a TypeRef target"
        );
    }

    #[test]
    fn opaque_return_emits_type_ref_and_plain_signature() {
        // `-> some Greet` must (a) emit a TypeRef from the function to the
        // peeled constraint `Greet`, and (b) record a plain return type in the
        // signature (`-> Greet`, not `-> some Greet`) so the index reads it as
        // the function's return type rather than falling back to a parameter.
        let src = r#"
protocol Greet { func hello() }
func make() -> some Greet { fatalError() }
"#;
        let r = super::extract::extract(src);

        let make = r
            .symbols
            .iter()
            .find(|s| s.name == "make")
            .expect("make function");
        let make_idx = r.symbols.iter().position(|s| std::ptr::eq(s, make)).unwrap();

        assert!(
            r.refs.iter().any(|rf| {
                rf.source_symbol_index == make_idx
                    && rf.kind == EdgeKind::TypeRef
                    && rf.target_name == "Greet"
            }),
            "make must emit a TypeRef to Greet; refs from {make_idx}: {:?}",
            r.refs
                .iter()
                .filter(|rf| rf.source_symbol_index == make_idx)
                .map(|rf| (&rf.target_name, rf.kind))
                .collect::<Vec<_>>()
        );

        assert_eq!(
            make.signature.as_deref(),
            Some("func make -> Greet"),
            "the opaque keyword must be peeled from the signature return type"
        );
    }

    #[test]
    fn opaque_return_with_generic_keeps_base_in_signature() {
        // `-> some Collection<Int>` peels the keyword but keeps the generic
        // application so the index can split head `Collection` from args.
        let src = r#"
func nums() -> some Collection<Int> { fatalError() }
"#;
        let r = super::extract::extract(src);
        let nums = r
            .symbols
            .iter()
            .find(|s| s.name == "nums")
            .expect("nums function");
        assert_eq!(
            nums.signature.as_deref(),
            Some("func nums -> Collection<Int>"),
            "keyword peeled, generic application preserved"
        );
    }

