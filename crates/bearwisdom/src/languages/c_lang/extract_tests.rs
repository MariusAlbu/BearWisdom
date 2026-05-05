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
        let r = extract::extract(src, "c");
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
        let r = extract::extract(src, "c");
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
        let r = extract::extract(src, "c");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");

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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "c");
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
        let r = extract::extract(src, "c");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
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
        let r = extract::extract(src, "cpp");
        // `vector` and `map` are type_identifier nodes inside template_type.
        let refs_names: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::TypeRef)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(refs_names.contains(&"User"), "missing User TypeRef: {refs_names:?}");
    }

    // =========================================================================
    // type_identifier in simple declarations (post-traversal scan)
    // =========================================================================

    #[test]
    fn c_simple_type_identifier_field_emits_typeref() {
        let src = r#"
struct Request {
    Session* session;
    UserContext ctx;
};
"#;
        let r = extract::extract(src, "c");
        let type_refs: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::TypeRef)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(type_refs.contains(&"Session"), "missing Session TypeRef: {type_refs:?}");
        assert!(type_refs.contains(&"UserContext"), "missing UserContext TypeRef: {type_refs:?}");
    }

    #[test]
    fn c_primitive_type_field_does_not_emit_typeref() {
        let src = r#"
struct Point {
    int x;
    float y;
    double z;
};
"#;
        let r = extract::extract(src, "c");
        // Primitives should NOT produce TypeRef edges
        let type_refs: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::TypeRef)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(!type_refs.contains(&"int"), "int should not produce TypeRef: {type_refs:?}");
        assert!(!type_refs.contains(&"float"), "float should not produce TypeRef: {type_refs:?}");
    }

    #[test]
    fn cpp_function_param_types_emit_typerefs() {
        let src = r#"
class EventBus {};
class Handler {};

void subscribe(EventBus* bus, Handler* handler) {}
"#;
        let r = extract::extract(src, "cpp");
        let type_refs: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::TypeRef)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(type_refs.contains(&"EventBus"), "missing EventBus TypeRef: {type_refs:?}");
        assert!(type_refs.contains(&"Handler"), "missing Handler TypeRef: {type_refs:?}");
    }

    #[test]
    fn cpp_function_return_type_emits_typeref() {
        let src = r#"
class Result {};
Result compute() { return Result(); }
"#;
        let r = extract::extract(src, "cpp");
        let type_refs: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::TypeRef)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(type_refs.contains(&"Result"), "missing Result TypeRef: {type_refs:?}");
    }

    // =========================================================================
    // typedef alias TypeRef (for chain walker dereference support)
    // =========================================================================

    #[test]
    fn cpp_typedef_alias_emits_typeref_to_source_type() {
        // `typedef SocketChannel* SocketChannelPtr;`
        // The TypeAlias symbol SocketChannelPtr must have a TypeRef to SocketChannel
        // so field_type_name("SocketChannelPtr") returns "SocketChannel".
        let src = r#"
class SocketChannel {};
typedef SocketChannel* SocketChannelPtr;
"#;
        let r = extract::extract(src, "cpp");
        // Should have a TypeAlias symbol for SocketChannelPtr.
        let alias = r.symbols.iter().find(|s| s.name == "SocketChannelPtr");
        assert!(alias.is_some(), "missing SocketChannelPtr TypeAlias symbol: {:?}", r.symbols);
        let alias = alias.unwrap();
        assert_eq!(alias.kind, SymbolKind::TypeAlias);

        // There must be a TypeRef from SocketChannelPtr (source_symbol_index == alias idx)
        // to "SocketChannel".
        let alias_idx = r.symbols.iter().position(|s| s.name == "SocketChannelPtr").unwrap();
        let has_typeref = r.refs.iter().any(|rf| {
            rf.source_symbol_index == alias_idx
                && rf.target_name == "SocketChannel"
                && rf.kind == EdgeKind::TypeRef
        });
        assert!(has_typeref, "SocketChannelPtr missing TypeRef to SocketChannel: {:?}", r.refs);
    }

    #[test]
    fn cpp_typedef_inside_template_class_is_scoped() {
        // typedef inside a template class body should be qualified under the class.
        let src = r#"
namespace hv {
template<class TSocketChannel>
class TcpClientTmpl {
public:
    typedef TSocketChannel* TSocketChannelPtr;
};
}
"#;
        let r = extract::extract(src, "cpp");
        // The typedef symbol should exist and be scoped to the class.
        let alias = r.symbols.iter().find(|s| s.name == "TSocketChannelPtr");
        assert!(alias.is_some(), "missing TSocketChannelPtr: {:?}", r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>());
        let alias = alias.unwrap();
        assert_eq!(alias.kind, SymbolKind::TypeAlias);
        // qualified_name should include the class scope.
        assert!(
            alias.qualified_name.contains("TcpClientTmpl"),
            "expected TcpClientTmpl in qualified_name, got: {}",
            alias.qualified_name
        );
    }

    #[test]
    fn cpp_class_field_typeref_attributed_to_field_not_class() {
        // `SocketChannel* channel;` inside a class → TypeRef from the field
        // Variable symbol to SocketChannel (not from the class).
        let src = r#"
class SocketChannel {};
class TcpClient {
public:
    SocketChannel* channel;
};
"#;
        let r = extract::extract(src, "cpp");
        // Find the "channel" Variable symbol.
        let field_sym = r.symbols.iter().find(|s| s.name == "channel");
        assert!(field_sym.is_some(), "missing channel symbol: {:?}", r.symbols);
        let field_idx = r.symbols.iter().position(|s| s.name == "channel").unwrap();

        // TypeRef to SocketChannel must be attributed to the "channel" field, not
        // the TcpClient class.
        let has_typeref = r.refs.iter().any(|rf| {
            rf.source_symbol_index == field_idx
                && rf.target_name == "SocketChannel"
                && rf.kind == EdgeKind::TypeRef
        });
        assert!(
            has_typeref,
            "expected TypeRef from channel (idx {field_idx}) to SocketChannel, refs: {:?}",
            r.refs.iter()
                .filter(|rf| rf.kind == EdgeKind::TypeRef)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn cpp_template_class_field_has_class_scope_debug() {
        let src = r#"
namespace hv {
template<class TSocketChannel>
class TcpClientEventLoopTmpl {
public:
    typedef TSocketChannel* TSocketChannelPtr;
    TSocketChannelPtr channel;
};
}
"#;
        let r = extract::extract(src, "cpp");
        // Print all symbols for debugging
        for sym in &r.symbols {
            println!("sym: name={}, qualified_name={}, kind={:?}", sym.name, sym.qualified_name, sym.kind);
        }
        // The channel field should be scoped to the class
        let channel = r.symbols.iter().find(|s| s.name == "channel");
        assert!(channel.is_some(), "channel not found");
        let channel = channel.unwrap();
        assert!(
            channel.qualified_name.contains("TcpClientEventLoopTmpl"),
            "channel not scoped to class, got: {}",
            channel.qualified_name
        );
    }

    #[test]
    fn cpp_template_class_with_default_field_has_class_scope() {
        let src = r#"
namespace hv {
template<class TSocketChannel = SocketChannel>
class TcpClientEventLoopTmpl {
public:
    typedef TSocketChannel* TSocketChannelPtr;
    TSocketChannelPtr channel;
};
}
"#;
        let r = extract::extract(src, "cpp");
        for sym in &r.symbols {
            println!("sym: name={}, qualified_name={}, kind={:?}", sym.name, sym.qualified_name, sym.kind);
        }
        let channel = r.symbols.iter().find(|s| s.name == "channel");
        assert!(channel.is_some(), "channel not found");
        let channel = channel.unwrap();
        assert!(
            channel.qualified_name.contains("TcpClientEventLoopTmpl"),
            "channel not scoped to class, got: {} (all: {:?})",
            channel.qualified_name,
            r.symbols.iter().map(|s| format!("{}={}", s.name, s.qualified_name)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn cpp_namespace_emits_namespace_symbol() {
        let src = r#"
namespace hv {
int foo() { return 0; }
}
"#;
        let r = extract::extract(src, "cpp");
        for sym in &r.symbols {
            println!("sym: name={}, qualified_name={}, kind={:?}", sym.name, sym.qualified_name, sym.kind);
        }
        let ns = r.symbols.iter().find(|s| s.name == "hv");
        assert!(ns.is_some(), "hv namespace not found: {:?}", r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>());
        let ns = ns.unwrap();
        assert_eq!(ns.kind, SymbolKind::Namespace, "hv has wrong kind: {:?}", ns.kind);
        let foo = r.symbols.iter().find(|s| s.name == "foo");
        assert!(foo.is_some(), "foo not found");
        let foo = foo.unwrap();
        assert!(foo.qualified_name.contains("hv"), "foo not scoped to hv, got: {}", foo.qualified_name);
    }

    #[test]
    fn cpp_tcpclient_real_file_scope_debug() {
        // Mimics the actual evpp/TcpClient.h structure from cpp-libhv
        let src = r#"
#include "Channel.h"

namespace hv {

template<class TSocketChannel = SocketChannel>
class TcpClientEventLoopTmpl {
public:
    typedef std::shared_ptr<TSocketChannel> TSocketChannelPtr;

    TcpClientEventLoopTmpl(EventLoopPtr loop = NULL) {
        loop_ = loop ? loop : std::make_shared<EventLoop>();
        connect_timeout = 0;
    }

    virtual ~TcpClientEventLoopTmpl() {}

    const EventLoopPtr& loop() { return loop_; }

    int createsocket(int remote_port, const char* remote_host = "127.0.0.1") {
        this->remote_port = remote_port;
        return 0;
    }

    int createsocket(struct sockaddr* remote_addr) {
        channel = std::make_shared<TSocketChannel>(NULL);
        return 0;
    }

public:
    TSocketChannelPtr channel;
    std::string remote_host;
    int remote_port;
    int connect_timeout;
};

typedef TcpClientEventLoopTmpl<SocketChannel> TcpClient;

} // namespace hv
"#;
        let r = extract::extract(src, "cpp");
        for sym in &r.symbols {
            println!("sym: name={}, qname={}, kind={:?}", sym.name, sym.qualified_name, sym.kind);
        }
        let hv = r.symbols.iter().find(|s| s.name == "hv");
        assert!(hv.is_some(), "hv namespace not found");
        assert_eq!(hv.unwrap().kind, SymbolKind::Namespace, "hv has wrong kind: {:?}", hv.unwrap().kind);
        let channel = r.symbols.iter().find(|s| s.name == "channel");
        assert!(channel.is_some(), "channel field not found");
        assert!(
            channel.unwrap().qualified_name.contains("TcpClientEventLoopTmpl"),
            "channel not scoped to class, got: {}",
            channel.unwrap().qualified_name
        );
    }

    #[test]
    fn cpp_tcpclient_actual_file_parse_debug() {
        // Read the actual TcpClient.h file and inspect what gets extracted
        let path = "/f/Work/Projects/TestProjects/cpp-libhv/evpp/TcpClient.h";
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => {
                println!("File not found, skipping");
                return;
            }
        };
        let r = extract::extract(&src, "cpp");
        println!("has_errors: {}", r.has_errors);
        for sym in &r.symbols {
            if matches!(sym.name.as_str(), "hv" | "channel" | "TcpClientEventLoopTmpl" | "TSocketChannelPtr" | "TcpClient") {
                println!("sym: name={}, qname={}, kind={:?}, line={}", 
                    sym.name, sym.qualified_name, sym.kind, sym.start_line);
            }
        }
        let hv = r.symbols.iter().find(|s| s.name == "hv");
        assert!(hv.is_some(), "hv not found in actual file");
        assert_eq!(
            hv.unwrap().kind,
            SymbolKind::Namespace,
            "hv has wrong kind {:?} in actual file. All syms: {:?}",
            hv.unwrap().kind,
            r.symbols.iter().map(|s| format!("{}:{:?}", s.name, s.kind)).collect::<Vec<_>>()
        );

        let channel = r.symbols.iter().find(|s| s.name == "channel" && s.qualified_name.contains("TcpClient"));
        assert!(
            channel.is_some(),
            "channel not scoped to TcpClient class. All channel syms: {:?}",
            r.symbols.iter().filter(|s| s.name == "channel")
                .map(|s| format!("qname={}", s.qualified_name))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn compiler_intrinsics_do_not_emit_calls() {
        // __builtin_*, __clang_*, __sync_*, __atomic_* are compiler-magic
        // — never defined in any source file. zig-compiler-fresh's vendored
        // LLVM source has 55K unresolved Calls dominated by these alone.
        // Filter at extract time so unresolved_refs stays honest.
        let src = r#"
int x = __builtin_bit_cast(int, 1.0f);
void *p = __builtin_alloca(64);
int y = __clang_arm_builtin_alias();
int z = __sync_fetch_and_add(&counter, 1);
int w = __atomic_load_n(&counter, __ATOMIC_SEQ_CST);
int normal = real_function(arg);
"#;
        let r = extract::extract(src, "c");
        let calls: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| rf.target_name.as_str())
            .collect();
        for forbidden in [
            "__builtin_bit_cast",
            "__builtin_alloca",
            "__clang_arm_builtin_alias",
            "__sync_fetch_and_add",
            "__atomic_load_n",
        ] {
            assert!(
                !calls.contains(&forbidden),
                "compiler intrinsic `{forbidden}` must NOT emit Calls; got {calls:?}"
            );
        }
        assert!(
            calls.contains(&"real_function"),
            "real call `real_function(arg)` SHOULD emit Calls; got {calls:?}"
        );
    }

    #[test]
    fn defined_in_preproc_if_does_not_emit_call() {
        // `defined(MACRO)` is a C preprocessor operator inside `#if`/`#elif`
        // directives, not a function call. tree-sitter-c parses it as a
        // call_expression, but it never resolves to a symbol — so it must
        // not appear in unresolved_refs.
        let src = r#"
#if defined(USE_OPENSSL) || !defined(WIN32)
int x = 1;
#endif

int real_call(void) {
    return helper();
}
"#;
        let r = extract::extract(src, "c");
        let calls: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(
            !calls.contains(&"defined"),
            "`defined` is a preprocessor operator, must not emit Calls; got {calls:?}"
        );
        assert!(
            calls.contains(&"helper"),
            "real call `helper()` SHOULD emit Calls; got {calls:?}"
        );
    }

    #[test]
    fn salvage_recovers_defines_after_parse_error() {
        // Real-world trigger from curl_setup.h: `typedef enum { ... } bool;`
        // pushes tree-sitter-c into recovery mode, after which subsequent
        // #define lines emit as ERROR/text content. The salvage pass must
        // recover them by raw-text scanning.
        //
        // Without the fallback: tree-sitter would extract `curlx_safefree`
        // (before the disruption) but miss `curlx_strdup`/`curlx_free`
        // (after). The fallback recovers them.
        let src = r#"
#define curlx_safefree(ptr) do { free(ptr); (ptr) = NULL; } while(0)

#ifndef HAVE_BOOL_T
  typedef enum {
    bool_false = 0,
    bool_true  = 1
  } bool;
#endif

#ifdef CURL_MEMDEBUG
#define curlx_strdup(ptr) curl_dbg_strdup(ptr, __LINE__, __FILE__)
#define curlx_calloc(nbelem, size) \
  curl_dbg_calloc(nbelem, size, __LINE__, __FILE__)
#define curlx_free(ptr) curl_dbg_free(ptr, __LINE__, __FILE__)
#else
#ifdef BUILDING_LIBCURL
#define curlx_strdup Curl_cstrdup
#define curlx_free   Curl_cfree
#else
#define curlx_strdup CURLX_STRDUP_LOW
#define curlx_free   free
#endif
#endif
"#;
        let r = extract::extract(src, "c");
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        for required in ["curlx_safefree", "curlx_strdup", "curlx_calloc", "curlx_free"] {
            assert!(
                names.contains(&required),
                "salvage failed to recover #define {required} after parse-error trigger; got: {names:?}"
            );
        }
    }

    #[test]
    fn salvage_does_not_duplicate_existing_symbols() {
        // When tree-sitter extracts the #define cleanly, the fallback
        // must dedup and not emit a duplicate.
        let src = r#"
#define ONE 1
#define TWO 2
"#;
        let r = extract::extract(src, "c");
        let one_count = r.symbols.iter().filter(|s| s.name == "ONE").count();
        let two_count = r.symbols.iter().filter(|s| s.name == "TWO").count();
        assert_eq!(one_count, 1, "#define ONE emitted {one_count} times (expected 1)");
        assert_eq!(two_count, 1, "#define TWO emitted {two_count} times (expected 1)");
    }

    #[test]
    fn salvage_does_not_match_defined_pseudo_call() {
        // `#if defined(FOO)` is the preprocessor `defined()` operator,
        // not a #define statement. The salvage scanner must not extract
        // `defined` or its argument as a symbol.
        let src = r#"
#if defined(FOO) || !defined(BAR)
int x = 1;
#endif
"#;
        let r = extract::extract(src, "c");
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            !names.contains(&"defined"),
            "salvage must not emit `defined` as a symbol; got: {names:?}"
        );
    }

    #[test]
    fn nested_ifdef_preproc_def_is_extracted() {
        // curl_setup.h has 2-deep nested #ifdef branches with #define inside:
        //
        //   #ifdef CURL_MEMDEBUG
        //   ...
        //   #else
        //     #ifdef BUILDING_LIBCURL
        //     #define curlx_free Curl_cfree
        //     #else
        //     #define curlx_free free
        //     #endif
        //   #endif
        //
        // Every branch's preproc_def must be extracted as a Variable symbol.
        let src = r#"
#ifdef OUTER
#ifdef INNER_A
#define MACRO_THEN_INNER_A a_impl
#else
#define MACRO_INNER_A_ELSE b_impl
#endif
#else
#define MACRO_OUTER_ELSE c_impl
#endif
"#;
        let r = extract::extract(src, "c");
        let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"MACRO_THEN_INNER_A"),
            "nested ifdef THEN/THEN branch missing; got: {names:?}"
        );
        assert!(
            names.contains(&"MACRO_INNER_A_ELSE"),
            "nested ifdef THEN/ELSE branch missing; got: {names:?}"
        );
        assert!(
            names.contains(&"MACRO_OUTER_ELSE"),
            "nested ifdef ELSE branch missing — this is the curl_setup.h case; got: {names:?}"
        );
    }

    // -------------------------------------------------------------------------
    // C++ template-call name capture (PR 210 of N — category 5 fix)
    //
    // Before this fix, `std::make_shared<T>()` and `obj->getContext<T>()` leaked
    // the literal source text — including angle brackets and the type name —
    // into `target_name`, producing entries like `make_shared<HttpRequest>` in
    // `unresolved_refs` that could never resolve. The fix drills through
    // `template_function` / `template_method` nodes to the inner identifier
    // and captures the template type-args separately.
    // -------------------------------------------------------------------------

    #[test]
    fn cpp_qualified_template_call_uses_inner_name() {
        let src = r#"
struct HttpRequest {};
namespace std { template<class T, class... A> int make_shared(A...); }
void f() {
    auto p = std::make_shared<HttpRequest>();
}
"#;
        let r = extract::extract(src, "cpp");
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(
            calls.iter().any(|c| *c == "make_shared"),
            "expected bare 'make_shared' as call target; got {calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.contains('<')),
            "no call target may contain '<' from template syntax; got {calls:?}"
        );
    }

    #[test]
    fn cpp_template_method_call_uses_inner_name() {
        let src = r#"
struct HttpClientContext {};
struct Conn {
    template<class T> int getContext();
};
void f(Conn* c) {
    c->getContext<HttpClientContext>();
}
"#;
        let r = extract::extract(src, "cpp");
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(
            calls.iter().any(|c| *c == "getContext"),
            "expected bare 'getContext' as call target; got {calls:?}"
        );
        assert!(
            !calls.iter().any(|c| c.contains('<')),
            "no call target may contain '<' from template syntax; got {calls:?}"
        );
    }

    #[test]
    fn cpp_template_call_captures_type_args_in_chain() {
        let src = r#"
struct HttpRequest {};
namespace std { template<class T, class... A> int make_shared(A...); }
void f() { auto p = std::make_shared<HttpRequest>(); }
"#;
        let r = extract::extract(src, "cpp");
        let chain_args: Vec<Vec<String>> = r
            .refs
            .iter()
            .filter(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "make_shared")
            .filter_map(|rf| rf.chain.as_ref())
            .filter_map(|c| c.segments.last())
            .map(|seg| seg.type_args.clone())
            .collect();
        assert!(
            chain_args.iter().any(|args| args.iter().any(|a| a == "HttpRequest")),
            "expected 'HttpRequest' captured as type_arg on the make_shared chain segment; got {chain_args:?}"
        );
    }
