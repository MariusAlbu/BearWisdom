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
        let r = extract::extract(src);
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
        let r = extract::extract(src);
        let en = r.symbols.iter().find(|s| s.name == "Direction").expect("Direction");
        assert_eq!(en.kind, SymbolKind::Enum);
        // The extractor should produce at least the enum itself; members depend on grammar version.
        assert!(!r.symbols.is_empty());
    }

    #[test]
    fn typedef_extracted_as_type_alias() {
        let src = r#"
typedef StringCallback = void Function(String value);
typedef JsonMap = Map<String, dynamic>;
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "StringCallback" && s.kind == SymbolKind::TypeAlias),
            "StringCallback TypeAlias not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.symbols.iter().any(|s| s.name == "JsonMap" && s.kind == SymbolKind::TypeAlias),
            "JsonMap TypeAlias not found"
        );
    }

    #[test]
    fn getter_setter_extracted_as_methods() {
        let src = r#"
class Config {
  String get name => _name;
  set name(String value) { _name = value; }
}
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "name" && s.kind == SymbolKind::Method),
            "name getter/setter not found; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn import_directive_produces_import_ref() {
        let src = "import 'dart:core';\nimport 'package:flutter/material.dart';\n";
        let r = extract::extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        assert!(!imports.is_empty(), "expected import refs");
    }

    #[test]
    fn library_prefix_dropped_across_usage_kinds() {
        // Generated drift code: `import '...' as i0;` then `i0.X` in type,
        // field, inheritance, and instantiation positions. The bare prefix
        // `i0` is a namespace anchor, not a reference, and must not appear as
        // a usage ref in any kind.
        let src = r#"
import 'package:drift/drift.dart' as i0;

class Foo extends i0.BaseReferences {
  i0.Value<String> field;
  void m() {
    var x = i0.GeneratedColumn();
  }
}
"#;
        let r = extract::extract(src);
        let prefix_usage: Vec<_> = r
            .refs
            .iter()
            .filter(|rf| rf.target_name == "i0" && rf.kind != EdgeKind::Imports)
            .collect();
        assert!(
            prefix_usage.is_empty(),
            "bare library prefix i0 must not be a usage ref; got {prefix_usage:?}"
        );
        assert!(
            r.refs.iter().any(|rf| rf.target_name == "BaseReferences" || rf.target_name == "Value"),
            "the qualified type after the prefix should be kept; refs: {:?}",
            r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
        );
    }
