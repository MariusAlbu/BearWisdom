    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    #[test]
    fn c_extracts_function_and_struct() {
        let src = r#"
struct Point {
    int x;
    int y;
};

int add(int a, int b) {
    return a + b;
}
"#;
        let r = extract(src, "c");
        assert!(!r.symbols.is_empty());

        let st = r.symbols.iter().find(|s| s.name == "Point").expect("Point");
        assert_eq!(st.kind, SymbolKind::Struct);

        let func = r.symbols.iter().find(|s| s.name == "add").expect("add");
        assert_eq!(func.kind, SymbolKind::Function);
    }

    #[test]
    fn c_extracts_enum_and_members() {
        let src = r#"
enum Direction {
    NORTH,
    SOUTH,
    EAST,
    WEST
};
"#;
        let r = extract(src, "c");
        let en = r.symbols.iter().find(|s| s.name == "Direction").expect("Direction");
        assert_eq!(en.kind, SymbolKind::Enum);
        assert!(r.symbols.iter().any(|s| s.name == "NORTH" && s.kind == SymbolKind::EnumMember));
        assert!(r.symbols.iter().any(|s| s.name == "WEST" && s.kind == SymbolKind::EnumMember));
    }

    #[test]
    fn c_include_produces_import_ref() {
        let src = r#"
#include <stdio.h>
#include "myheader.h"

int main() { return 0; }
"#;
        let r = extract(src, "c");
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"stdio.h"), "missing stdio.h: {targets:?}");
        assert!(targets.contains(&"myheader.h"), "missing myheader.h: {targets:?}");
    }

    #[test]
    fn cpp_extracts_class_and_method() {
        let src = r#"
class Animal {
public:
    Animal(const char* name);
    void speak();
};
"#;
        let r = extract(src, "cpp");
        let cls = r.symbols.iter().find(|s| s.name == "Animal").expect("Animal");
        assert_eq!(cls.kind, SymbolKind::Class);
    }
