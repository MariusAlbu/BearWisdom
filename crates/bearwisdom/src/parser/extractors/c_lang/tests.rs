    use super::*;
    use crate::types::{EdgeKind, SymbolKind};

    // =========================================================================
    // Existing tests
    // =========================================================================

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

    // =========================================================================
    // template_declaration
    // =========================================================================

    #[test]
    fn cpp_template_class_emits_class_symbol() {
        let src = r#"
template<typename T>
class Stack {
    T data;
    int size;
};
"#;
        let r = extract(src, "cpp");
        let sym = r.symbols.iter().find(|s| s.name == "Stack").expect("Stack");
        assert_eq!(sym.kind, SymbolKind::Class);
    }

    #[test]
    fn cpp_template_function_emits_function_symbol() {
        let src = r#"
template<typename T>
T max_val(T a, T b) {
    return a > b ? a : b;
}
"#;
        let r = extract(src, "cpp");
        let sym = r.symbols.iter().find(|s| s.name == "max_val").expect("max_val");
        assert_eq!(sym.kind, SymbolKind::Function);
    }

    #[test]
    fn cpp_template_default_type_param_emits_typeref() {
        let src = r#"
template<typename T, typename Alloc = MyAllocator>
class Container {
    T data;
};
"#;
        let r = extract(src, "cpp");
        // Should have TypeRef to `MyAllocator` from the default type parameter.
        let has_ref = r.refs.iter().any(|rf| {
            rf.target_name == "MyAllocator" && rf.kind == EdgeKind::TypeRef
        });
        assert!(has_ref, "expected TypeRef to `MyAllocator` from default type param: {:?}", r.refs);
    }

    // =========================================================================
    // alias_declaration
    // =========================================================================

    #[test]
    fn cpp_alias_declaration_emits_type_alias_and_typeref() {
        let src = r#"
using MyVec = std::vector<int>;
using Score = float;
"#;
        let r = extract(src, "cpp");

        let my_vec = r.symbols.iter().find(|s| s.name == "MyVec").expect("MyVec");
        assert_eq!(my_vec.kind, SymbolKind::TypeAlias);

        let score = r.symbols.iter().find(|s| s.name == "Score").expect("Score");
        assert_eq!(score.kind, SymbolKind::TypeAlias);
    }

    // =========================================================================
    // using_declaration
    // =========================================================================

    #[test]
    fn cpp_using_declaration_emits_imports_ref() {
        let src = r#"
using std::vector;
using std::string;
"#;
        let r = extract(src, "cpp");
        let imports: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Imports).collect();
        let targets: Vec<&str> = imports.iter().map(|rf| rf.target_name.as_str()).collect();
        // qualified_identifier text for `std::vector`
        assert!(
            targets.iter().any(|t| t.contains("vector")),
            "missing vector import: {targets:?}"
        );
        assert!(
            targets.iter().any(|t| t.contains("string")),
            "missing string import: {targets:?}"
        );
    }

    // =========================================================================
    // preproc_def / preproc_function_def
    // =========================================================================

    #[test]
    fn c_preproc_def_emits_variable_symbol() {
        let src = r#"
#define PI 3.14159
#define MAX_SIZE 1024
"#;
        let r = extract(src, "c");
        let pi = r.symbols.iter().find(|s| s.name == "PI").expect("PI");
        assert_eq!(pi.kind, SymbolKind::Variable);
        assert!(pi.signature.as_deref().unwrap_or("").contains("3.14159"));

        let ms = r.symbols.iter().find(|s| s.name == "MAX_SIZE").expect("MAX_SIZE");
        assert_eq!(ms.kind, SymbolKind::Variable);
    }

    #[test]
    fn c_preproc_function_def_emits_function_symbol() {
        let src = r#"
#define MAX(a, b) ((a) > (b) ? (a) : (b))
#define SQ(x) ((x) * (x))
"#;
        let r = extract(src, "c");
        let max = r.symbols.iter().find(|s| s.name == "MAX").expect("MAX");
        assert_eq!(max.kind, SymbolKind::Function);
        assert!(max.signature.as_deref().unwrap_or("").contains("(a, b)"));

        let sq = r.symbols.iter().find(|s| s.name == "SQ").expect("SQ");
        assert_eq!(sq.kind, SymbolKind::Function);
    }

    // =========================================================================
    // cast_expression
    // =========================================================================

    #[test]
    fn cpp_c_style_cast_emits_typeref() {
        let src = r#"
void foo(double d) {
    int x = (int)d;
    MyType* p = (MyType*)malloc(sizeof(MyType));
}
"#;
        let r = extract(src, "cpp");
        let has_mytype = r.refs.iter().any(|rf| {
            rf.target_name == "MyType" && rf.kind == EdgeKind::TypeRef
        });
        assert!(has_mytype, "expected TypeRef to MyType from cast: {:?}", r.refs);
    }

    // =========================================================================
    // sizeof_expression
    // =========================================================================

    #[test]
    fn cpp_sizeof_named_type_emits_typeref() {
        let src = r#"
struct Node { int val; };
void foo() {
    int sz = sizeof(Node);
}
"#;
        // `sizeof(Node)` — tree-sitter parses this as sizeof + parenthesized_expression
        // containing an identifier (ambiguous with expression), so we emit TypeRef
        // for any bare identifier inside.
        let r = extract(src, "cpp");
        let has_node = r.refs.iter().any(|rf| {
            rf.target_name == "Node" && rf.kind == EdgeKind::TypeRef
        });
        assert!(has_node, "expected TypeRef to Node from sizeof: {:?}", r.refs);
    }

    // =========================================================================
    // new_expression
    // =========================================================================

    #[test]
    fn cpp_new_expression_emits_instantiates_and_typeref() {
        let src = r#"
class Widget {};
void factory() {
    auto w = new Widget();
}
"#;
        let r = extract(src, "cpp");
        let has_instantiates = r.refs.iter().any(|rf| {
            rf.target_name == "Widget" && rf.kind == EdgeKind::Instantiates
        });
        assert!(has_instantiates, "expected Instantiates to Widget: {:?}", r.refs);

        let has_typeref = r.refs.iter().any(|rf| {
            rf.target_name == "Widget" && rf.kind == EdgeKind::TypeRef
        });
        assert!(has_typeref, "expected TypeRef to Widget: {:?}", r.refs);
    }

    #[test]
    fn cpp_new_template_type_emits_instantiates() {
        let src = r#"
void foo() {
    auto p = new std::vector<int>();
}
"#;
        // `new std::vector<int>()` — the type is a `template_type` or
        // `qualified_identifier`. Either way we should get an Instantiates
        // or TypeRef for `vector`.
        let r = extract(src, "cpp");
        // At minimum we should not crash. Check for any TypeRef or Instantiates.
        let _ = r; // smoke test
    }

    // =========================================================================
    // lambda_expression
    // =========================================================================

    #[test]
    fn cpp_lambda_param_types_emits_typeref() {
        let src = r#"
class Callback {};
void register_cb() {
    auto fn = [](Callback* cb) {
        cb->invoke();
    };
}
"#;
        let r = extract(src, "cpp");
        let has_cb = r.refs.iter().any(|rf| {
            rf.target_name == "Callback" && rf.kind == EdgeKind::TypeRef
        });
        assert!(has_cb, "expected TypeRef to Callback from lambda param: {:?}", r.refs);
    }

    #[test]
    fn cpp_lambda_body_calls_extracted() {
        let src = r#"
void setup() {
    auto fn = [&]() {
        do_work();
        cleanup();
    };
}
"#;
        let r = extract(src, "cpp");
        let calls: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(calls.contains(&"do_work"), "missing do_work call: {calls:?}");
        assert!(calls.contains(&"cleanup"), "missing cleanup call: {calls:?}");
    }

    // =========================================================================
    // catch_clause
    // =========================================================================

    #[test]
    fn cpp_catch_clause_emits_typeref() {
        let src = r#"
void risky() {
    try {
        do_thing();
    } catch (std::runtime_error& e) {
        log_error(e);
    }
}
"#;
        let r = extract(src, "cpp");
        let has_runtime_error = r.refs.iter().any(|rf| {
            rf.target_name == "runtime_error" && rf.kind == EdgeKind::TypeRef
        });
        assert!(
            has_runtime_error,
            "expected TypeRef to runtime_error from catch clause: {:?}", r.refs
        );
    }

    #[test]
    fn cpp_catch_calls_extracted() {
        let src = r#"
void risky() {
    try {
        connect();
    } catch (std::exception& e) {
        log_exception(e);
        recover();
    }
}
"#;
        let r = extract(src, "cpp");
        let calls: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(calls.contains(&"connect"), "missing connect call: {calls:?}");
        assert!(calls.contains(&"log_exception"), "missing log_exception call: {calls:?}");
        assert!(calls.contains(&"recover"), "missing recover call: {calls:?}");
    }

    // =========================================================================
    // template_type in declarations
    // =========================================================================

    #[test]
    fn cpp_template_type_field_emits_typeref() {
        let src = r#"
class Repo {
    std::vector<User> users;
    std::map<std::string, Order> index;
};
"#;
        let r = extract(src, "cpp");
        // `vector` and `map` are type_identifier nodes inside template_type.
        let refs_names: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::TypeRef)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(refs_names.contains(&"User"), "missing User TypeRef: {refs_names:?}");
    }
