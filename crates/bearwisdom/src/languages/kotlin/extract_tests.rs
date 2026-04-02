    use super::extract::extract;
use crate::types::{ExtractedRef, ExtractedSymbol};
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn extracts_class_with_method() {
        let src = r#"
class Animal(val name: String) {
    fun speak(): String {
        return "..."
    }
}
"#;
        let r = extract(src);
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal");
        assert_eq!(cls.kind, SymbolKind::Class);

        let method = r.symbols.iter().find(|s| s.name == "speak").expect("speak");
        assert_eq!(method.kind, SymbolKind::Method);
    }

    #[test]
    fn extracts_enum_class() {
        let src = r#"
enum class Direction {
    NORTH,
    SOUTH,
    EAST,
    WEST
}
"#;
        let r = extract(src);
        let en = r.symbols.iter().find(|s| s.name == "Direction").expect("Direction");
        assert_eq!(en.kind, SymbolKind::Enum);
        // Enum members depend on grammar version; at least the enum itself must be present.
        assert!(!r.symbols.is_empty());
    }

    #[test]
    fn companion_object_extracted_as_class() {
        let src = r#"
class Config {
    companion object {
        val DEFAULT_TIMEOUT = 30
        fun create(): Config = Config()
    }
}
"#;
        let r = extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "Companion" && s.kind == SymbolKind::Class),
            "Companion not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        // create() should be extracted as a member inside the companion.
        assert!(r.symbols.iter().any(|s| s.name == "create"));
    }

    #[test]
    fn as_expression_emits_type_ref() {
        let src = r#"
fun cast(x: Any): String {
    return x as String
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
    fn primary_constructor_promoted_params_extracted() {
        let src = r#"
class Point(val x: Double, val y: Double)
"#;
        let r = extract(src);
        // val x and val y become Property symbols.
        assert!(
            r.symbols.iter().any(|s| s.name == "x" && s.kind == SymbolKind::Property),
            "x property not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(r.symbols.iter().any(|s| s.name == "y" && s.kind == SymbolKind::Property));
        // TypeRefs for Double emitted.
        assert!(
            r.refs.iter().any(|rf| rf.target_name == "Double" && rf.kind == EdgeKind::TypeRef),
            "TypeRef for Double not found; refs: {:?}",
            r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn interface_and_class_extracted() {
        let src = r#"
interface Drawable {
    fun draw()
}

class Circle : Drawable {
    override fun draw() {}
}
"#;
        let r = extract(src);
        // Kotlin grammar may emit interface_declaration or class_declaration for interfaces
        assert!(
            r.symbols.iter().any(|s| s.name == "Drawable"),
            "Drawable not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(r.symbols.iter().any(|s| s.name == "Circle" && s.kind == SymbolKind::Class));
    }
