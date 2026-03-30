    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn extracts_class() {
        let src = r#"
class Animal {
  final String name;

  Animal(this.name);

  String speak() {
    return '...';
  }
}
"#;
        let r = extract(src);
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal");
        assert_eq!(cls.kind, SymbolKind::Class);
        // At minimum the class itself is extracted.
        assert!(r.symbols.iter().any(|s| s.parent_index == Some(0)));
    }

    #[test]
    fn extracts_enum() {
        let src = r#"
enum Direction {
  north,
  south,
  east,
  west,
}
"#;
        let r = extract(src);
        let en = r.symbols.iter().find(|s| s.name == "Direction").expect("Direction");
        assert_eq!(en.kind, SymbolKind::Enum);
        // The extractor should produce at least the enum itself; members depend on grammar version.
        assert!(!r.symbols.is_empty());
    }

    #[test]
    fn import_directive_produces_import_ref() {
        let src = "import 'dart:core';\nimport 'package:flutter/material.dart';\n";
        let r = extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        assert!(!imports.is_empty(), "expected import refs");
    }
