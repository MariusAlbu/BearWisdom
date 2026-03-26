    use super::*;
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
