// =============================================================================
// java/coverage_tests.rs — One test per node kind in symbol_node_kinds()
// and ref_node_kinds() declared in java/mod.rs.
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

fn sym(src: &str) -> Vec<crate::types::ExtractedSymbol> {
    extract::extract(src).symbols
}
fn refs(src: &str) -> Vec<crate::types::ExtractedRef> {
    extract::extract(src).refs
}

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn coverage_class_declaration() {
    let src = "public class Foo {}";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "Foo" && s.kind == SymbolKind::Class),
        "expected Class symbol Foo; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_interface_declaration() {
    let src = "public interface IRepository {}";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "IRepository" && s.kind == SymbolKind::Interface),
        "expected Interface symbol IRepository; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_enum_declaration() {
    let src = "public enum Status { ACTIVE, INACTIVE }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "Status" && s.kind == SymbolKind::Enum),
        "expected Enum symbol Status; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_enum_constant() {
    let src = "public enum Status { ACTIVE, INACTIVE }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "ACTIVE" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember symbol ACTIVE; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        s.iter().any(|s| s.name == "INACTIVE" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember symbol INACTIVE; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_record_declaration() {
    let src = "public record Point(int x, int y) {}";
    let s = sym(src);
    // record maps to Class in the extractor
    assert!(
        s.iter().any(|s| s.name == "Point" && s.kind == SymbolKind::Class),
        "expected Class(record) symbol Point; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_annotation_type_declaration() {
    let src = "public @interface MyAnnotation { String value() default \"\"; }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "MyAnnotation" && s.kind == SymbolKind::Interface),
        "expected Interface symbol (annotation type) MyAnnotation; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_declaration() {
    let src = "class C { void doWork() {} }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "doWork" && s.kind == SymbolKind::Method),
        "expected Method symbol doWork; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_constructor_declaration() {
    let src = "class Service { public Service(String name) {} }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "Service" && s.kind == SymbolKind::Constructor),
        "expected Constructor symbol Service; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_compact_constructor_declaration() {
    // Java 16+ records have compact constructors (no parameter list)
    let src = "public record Point(int x, int y) { Point { if (x < 0) throw new IllegalArgumentException(); } }";
    let s = sym(src);
    // The record itself should be extracted; compact constructor may or may not emit a separate symbol
    assert!(
        s.iter().any(|s| s.name == "Point"),
        "expected Point symbol from record with compact constructor; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_field_declaration() {
    let src = "class C { private String name; }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "name" && s.kind == SymbolKind::Field),
        "expected Field symbol name; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_constant_declaration() {
    let src = "class C { public static final int MAX = 100; }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "MAX" && s.kind == SymbolKind::Field),
        "expected Field(constant) symbol MAX; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_annotation_type_element_declaration() {
    // annotation_type_element_declaration — elements inside @interface
    let src = "public @interface Config { String value() default \"\"; int timeout() default 30; }";
    let s = sym(src);
    // The annotation type itself should be extracted
    assert!(
        s.iter().any(|s| s.name == "Config"),
        "expected Config annotation type symbol; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_package_declaration() {
    let src = "package com.example.service;\nclass X {}";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.kind == SymbolKind::Namespace),
        "expected Namespace symbol from package_declaration; got: {:?}",
        s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    let ns = s.iter().find(|s| s.kind == SymbolKind::Namespace).unwrap();
    assert_eq!(ns.qualified_name, "com.example.service",
        "package qualified_name mismatch");
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn coverage_method_invocation() {
    let src = "class C { void m() { System.out.println(); } }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "println" && r.kind == EdgeKind::Calls),
        "expected Calls edge for println; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_object_creation_expression() {
    let src = "class C { void m() { new ArrayList(); } }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "ArrayList" && r.kind == EdgeKind::Instantiates),
        "expected Instantiates edge for ArrayList; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_import_declaration() {
    let src = "import java.util.List;";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "List" && r.kind == EdgeKind::Imports),
        "expected Imports edge for List; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_arguments() {
    // Generic type arguments should produce TypeRef edges
    let src = "class C { void m() { java.util.List<Widget> list = new java.util.ArrayList<>(); } }";
    let r = refs(src);
    // Should have Instantiates for ArrayList at minimum
    assert!(
        r.iter().any(|r| r.kind == EdgeKind::Instantiates),
        "expected Instantiates edge from object_creation_expression with type_arguments; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_instanceof_expression() {
    let src = "class C { void m(Object o) { if (o instanceof Admin) {} } }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
        "expected TypeRef edge from instanceof_expression for Admin; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_reference() {
    let src = "class C { void m() { users.stream().map(User::getName); } }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "getName" && r.kind == EdgeKind::Calls),
        "expected Calls edge from method_reference for getName; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_cast_expression() {
    let src = "class C { void m(Object o) { Admin a = (Admin) o; } }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
        "expected TypeRef edge from cast_expression for Admin; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_annotation_marker() {
    // marker_annotation (no arguments)
    let src = "@Service\npublic class UserService {}";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Service" && r.kind == EdgeKind::TypeRef),
        "expected TypeRef edge for @Service annotation; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_annotation_with_args() {
    // annotation (with arguments)
    let src = "public class C { @GetMapping(\"/users/{id}\") public void get() {} }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "GetMapping" && r.kind == EdgeKind::TypeRef),
        "expected TypeRef edge for @GetMapping annotation; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_superclass() {
    // superclass field on class_declaration → Inherits edge
    let src = "public class UserService extends BaseService {}";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "BaseService" && r.kind == EdgeKind::Inherits),
        "expected Inherits edge for BaseService (superclass); refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_super_interfaces() {
    // super_interfaces → class implements → Implements edges
    let src = "public class Svc extends Base implements Runnable, Serializable {}";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Runnable" && r.kind == EdgeKind::Implements),
        "expected Implements edge for Runnable (super_interfaces); refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.iter().any(|r| r.target_name == "Serializable" && r.kind == EdgeKind::Implements),
        "expected Implements edge for Serializable (super_interfaces); refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_extends_interfaces() {
    // extends_interfaces → interface extends → Implements edges
    let src = "public interface ExtendedRepo extends Repository, ReadOnly {}";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Repository" && r.kind == EdgeKind::Implements),
        "expected Implements edge for Repository (extends_interfaces); refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.iter().any(|r| r.target_name == "ReadOnly" && r.kind == EdgeKind::Implements),
        "expected Implements edge for ReadOnly (extends_interfaces); refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_field_declaration_type_ref() {
    // field_declaration with a generic type should emit TypeRef for both the base type and arg.
    let src = "import java.util.List;\nclass C { private List<User> users; }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "List" && r.kind == EdgeKind::TypeRef),
        "expected TypeRef for List from field_declaration; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
        "expected TypeRef for User (type argument) from field_declaration; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_field_declaration_annotation() {
    // Annotations on field declarations should produce TypeRef edges.
    let src = "class C { @Autowired private UserService userService; }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Autowired" && r.kind == EdgeKind::TypeRef),
        "expected TypeRef for @Autowired annotation on field; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_arguments_in_method_call() {
    // type_arguments inside a method_invocation: Collections.<String>emptyList()
    let src = "class C { void m() { java.util.Collections.<String>emptyList(); } }";
    let r = refs(src);
    // At minimum should produce a Calls edge.
    assert!(
        r.iter().any(|r| r.target_name == "emptyList" && r.kind == EdgeKind::Calls),
        "expected Calls edge for emptyList; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

// ---- field_declaration in anonymous class body → Field symbol ---------------

#[test]
fn coverage_field_declaration_in_anonymous_class_body_from_field_initializer() {
    // A field initialized with an anonymous class must have its field_declarations
    // inside the anonymous class body extracted as symbols.
    //
    // Previously `extract_nested_classes_from_body` was not called for field
    // initializers, so anonymous class fields were silently dropped.
    let src = r#"
class Outer {
    private Comparator<String> comparator = new Comparator<String>() {
        private int multiplier = 1;
        @Override
        public int compare(String a, String b) { return a.compareTo(b); }
    };
}
"#;
    let r = super::extract::extract(src);
    let syms: Vec<(&str, crate::types::SymbolKind)> = r
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.kind))
        .collect();
    assert!(
        syms.iter().any(|(n, k)| *n == "multiplier" && *k == crate::types::SymbolKind::Field),
        "expected Field symbol 'multiplier' from anonymous class body in field initializer; symbols: {:?}",
        syms
    );
    // The compare method inside the anonymous class should also be extracted.
    assert!(
        syms.iter().any(|(n, k)| *n == "compare" && *k == crate::types::SymbolKind::Method),
        "expected Method symbol 'compare' from anonymous class body in field initializer; symbols: {:?}",
        syms
    );
}

#[test]
fn coverage_field_declaration_in_anon_class_inside_constructor_args() {
    // An anonymous class passed as constructor argument must have its fields extracted.
    // `extract_nested_classes_from_body` previously missed anonymous classes nested
    // inside constructor arguments (non-class_body children of object_creation_expression).
    let src = r#"
class Outer {
    void setup() {
        Container c = new Container(new Listener() {
            private int count = 0;
            public void onEvent() {}
        });
    }
}
"#;
    let r = super::extract::extract(src);
    let syms: Vec<(&str, crate::types::SymbolKind)> = r
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.kind))
        .collect();
    assert!(
        syms.iter().any(|(n, k)| *n == "count" && *k == crate::types::SymbolKind::Field),
        "expected Field 'count' from anon class in constructor args; symbols: {:?}",
        syms
    );
    assert!(
        syms.iter().any(|(n, k)| *n == "onEvent" && *k == crate::types::SymbolKind::Method),
        "expected Method 'onEvent' from anon class in constructor args; symbols: {:?}",
        syms
    );
}

#[test]
fn coverage_method_invocation_in_lambda_body() {
    // Method calls inside lambda bodies must emit Calls edges.
    let src = r#"
class C {
    void m() {
        list.stream()
            .map(item -> item.process())
            .filter(item -> item.isValid())
            .forEach(item -> item.save());
    }
}
"#;
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "process" && r.kind == EdgeKind::Calls),
        "expected Calls 'process' from lambda body; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.iter().any(|r| r.target_name == "isValid" && r.kind == EdgeKind::Calls),
        "expected Calls 'isValid' from lambda body; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_invocation_in_ternary() {
    // Method calls in ternary branches must emit Calls edges.
    let src = r#"
class C {
    String m(boolean flag) {
        return flag ? service.getA() : service.getB();
    }
}
"#;
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "getA" && r.kind == EdgeKind::Calls),
        "expected Calls 'getA' from ternary true-branch; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.iter().any(|r| r.target_name == "getB" && r.kind == EdgeKind::Calls),
        "expected Calls 'getB' from ternary false-branch; refs: {:?}",
        r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
    );
}

#[test]
#[ignore]
fn debug_measure_coverage_java() {
    let projects = [
        "F:/Work/Projects/TestProjects/java-spring-petclinic",
        "F:/Work/Projects/TestProjects/java-petclinic-rest",
        "F:/Work/Projects/TestProjects/java-petclinic-reactjs",
        "F:/Work/Projects/TestProjects/java-spring-boot-admin",
        "F:/Work/Projects/TestProjects/java-recaf",
    ];
    let project_path = projects.iter().find(|p| std::path::Path::new(p).exists()).copied();
    let project_path = match project_path {
        Some(p) => p,
        None => { eprintln!("No Java test project found"); return; }
    };
    eprintln!("Using project: {}", project_path);
    let results = crate::query::coverage::analyze_coverage(std::path::Path::new(project_path));
    for cov in &results {
        if cov.language == "java" {
            eprintln!("=== Java ===");
            eprintln!("  files: {}", cov.file_count);
            eprintln!("  sym: {:.1}% ({}/{})", cov.symbol_coverage.percent, cov.symbol_coverage.matched_nodes, cov.symbol_coverage.expected_nodes);
            eprintln!("  ref: {:.1}% ({}/{})", cov.ref_coverage.percent, cov.ref_coverage.matched_nodes, cov.ref_coverage.expected_nodes);
            eprintln!("  --- symbol kinds (worst first) ---");
            let mut sym_kinds = cov.symbol_kinds.clone();
            sym_kinds.sort_by(|a, b| a.percent.partial_cmp(&b.percent).unwrap());
            for k in sym_kinds.iter().take(10) {
                eprintln!("    {}: {:.1}% ({}/{}) miss={}", k.kind, k.percent, k.matched, k.occurrences, k.occurrences - k.matched);
            }
            eprintln!("  --- ref kinds (worst first) ---");
            let mut ref_kinds = cov.ref_kinds.clone();
            ref_kinds.sort_by(|a, b| a.percent.partial_cmp(&b.percent).unwrap());
            for k in ref_kinds.iter().take(10) {
                eprintln!("    {}: {:.1}% ({}/{}) miss={}", k.kind, k.percent, k.matched, k.occurrences, k.occurrences - k.matched);
            }
        }
    }
}
