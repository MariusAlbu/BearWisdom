    use super::extract;
    use crate::types::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    fn sym(source: &str) -> Vec<ExtractedSymbol> {
        extract::extract(source).symbols
    }
    fn refs(source: &str) -> Vec<ExtractedRef> {
        extract::extract(source).refs
    }

    // -----------------------------------------------------------------------
    // 1. Class with methods and fields
    // -----------------------------------------------------------------------
    #[test]
    fn class_with_methods_and_fields() {
        let src = r#"
package com.example;

public class UserService {
    private String name;

    public String getName() { return name; }

    protected void setName(String name) { this.name = name; }
}
"#;
        let symbols = sym(src);

        let cls = symbols.iter().find(|s| s.name == "UserService").unwrap();
        assert_eq!(cls.kind, SymbolKind::Class);
        assert_eq!(cls.visibility, Some(Visibility::Public));
        assert_eq!(cls.qualified_name, "com.example.UserService");

        let field = symbols.iter().find(|s| s.name == "name" && s.kind == SymbolKind::Field).unwrap();
        assert_eq!(field.visibility, Some(Visibility::Private));
        assert!(field.signature.as_ref().unwrap().contains("String"));

        let get_name = symbols.iter().find(|s| s.name == "getName").unwrap();
        assert_eq!(get_name.kind, SymbolKind::Method);
        assert_eq!(get_name.visibility, Some(Visibility::Public));
        assert_eq!(get_name.qualified_name, "com.example.UserService.getName");

        let set_name = symbols.iter().find(|s| s.name == "setName").unwrap();
        assert_eq!(set_name.visibility, Some(Visibility::Protected));
    }

    // -----------------------------------------------------------------------
    // 2. Interface with default methods
    // -----------------------------------------------------------------------
    #[test]
    fn interface_with_default_method() {
        let src = r#"
package com.example;

public interface Repository<T> {
    T findById(long id);

    default void delete(long id) {}
}
"#;
        let symbols = sym(src);
        let iface = symbols.iter().find(|s| s.name == "Repository").unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert_eq!(iface.qualified_name, "com.example.Repository");

        assert!(symbols.iter().any(|s| s.name == "findById" && s.kind == SymbolKind::Method));
        assert!(symbols.iter().any(|s| s.name == "delete"    && s.kind == SymbolKind::Method));
    }

    // -----------------------------------------------------------------------
    // 3. Enum with constants
    // -----------------------------------------------------------------------
    #[test]
    fn enum_with_constants() {
        let src = r#"
package com.example;

public enum Status {
    PENDING,
    ACTIVE,
    DELETED;

    public boolean isActive() { return this == ACTIVE; }
}
"#;
        let symbols = sym(src);

        let e = symbols.iter().find(|s| s.name == "Status" && s.kind == SymbolKind::Enum).unwrap();
        assert_eq!(e.qualified_name, "com.example.Status");

        let members: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::EnumMember).collect();
        let names: Vec<&str> = members.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"PENDING"), "got: {names:?}");
        assert!(names.contains(&"ACTIVE"),  "got: {names:?}");
        assert!(names.contains(&"DELETED"), "got: {names:?}");

        // Method inside enum body.
        assert!(symbols.iter().any(|s| s.name == "isActive" && s.kind == SymbolKind::Method));
    }

    // -----------------------------------------------------------------------
    // 4. Nested / inner classes get qualified names
    // -----------------------------------------------------------------------
    #[test]
    fn nested_class_qualified_name() {
        let src = r#"
package com.example;

public class Outer {
    public static class Inner {
        public void work() {}
    }
}
"#;
        let symbols = sym(src);

        let outer = symbols.iter().find(|s| s.name == "Outer").unwrap();
        assert_eq!(outer.qualified_name, "com.example.Outer");

        let inner = symbols.iter().find(|s| s.name == "Inner").unwrap();
        // Inner should be qualified relative to Outer.
        assert!(
            inner.qualified_name.contains("Outer.Inner"),
            "expected Outer.Inner in qualified_name, got: {}",
            inner.qualified_name
        );

        let method = symbols.iter().find(|s| s.name == "work").unwrap();
        assert!(
            method.qualified_name.contains("Inner.work"),
            "expected Inner.work in qualified_name, got: {}",
            method.qualified_name
        );
    }

    // -----------------------------------------------------------------------
    // 5. Imports extracted correctly
    // -----------------------------------------------------------------------
    #[test]
    fn import_extracted() {
        let src = r#"
import java.util.List;
import java.util.ArrayList;
import static org.junit.Assert.*;
"#;
        let r = refs(src);
        let imports: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Imports).collect();

        let list_import = imports.iter().find(|i| i.target_name == "List");
        assert!(list_import.is_some(), "Missing List import; imports: {:?}",
            imports.iter().map(|i| &i.target_name).collect::<Vec<_>>());
        assert_eq!(list_import.unwrap().module.as_deref(), Some("java.util.List"));

        assert!(imports.iter().any(|i| i.target_name == "ArrayList"));
    }

    // -----------------------------------------------------------------------
    // 6. Method calls create Calls edges
    // -----------------------------------------------------------------------
    #[test]
    fn method_calls_create_edges() {
        let src = r#"
class Service {
    void run() {
        foo();
        bar.baz();
        String s = helper.doSomething();
    }
}
"#;
        let r = refs(src);
        let calls: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
        let names: Vec<&str> = calls.iter().map(|r| r.target_name.as_str()).collect();

        assert!(names.contains(&"foo"),         "Missing foo; calls: {names:?}");
        assert!(names.contains(&"baz"),         "Missing baz; calls: {names:?}");
        assert!(names.contains(&"doSomething"), "Missing doSomething; calls: {names:?}");
    }

    // -----------------------------------------------------------------------
    // 7. @Test annotation promotes method to SymbolKind::Test
    // -----------------------------------------------------------------------
    #[test]
    fn test_annotation_detected() {
        let src = r#"
import org.junit.jupiter.api.Test;

class CalculatorTest {
    @Test
    void addsTwoNumbers() {
        assert 1 + 1 == 2;
    }

    @ParameterizedTest
    void addsManyNumbers(int a, int b) {}

    void helperMethod() {}
}
"#;
        let symbols = sym(src);

        let adds = symbols.iter().find(|s| s.name == "addsTwoNumbers").unwrap();
        assert_eq!(adds.kind, SymbolKind::Test, "addsTwoNumbers should be Test");

        let parameterized = symbols.iter().find(|s| s.name == "addsManyNumbers").unwrap();
        assert_eq!(parameterized.kind, SymbolKind::Test, "addsManyNumbers should be Test");

        let helper = symbols.iter().find(|s| s.name == "helperMethod").unwrap();
        assert_eq!(helper.kind, SymbolKind::Method, "helperMethod should be Method");
    }

    // -----------------------------------------------------------------------
    // 8. Inheritance and implementation create correct edge kinds
    // -----------------------------------------------------------------------
    #[test]
    fn inheritance_edges() {
        let src = r#"
package com.example;

public class UserService extends BaseService implements Serializable, Cloneable {}
"#;
        let r = refs(src);

        assert!(
            r.iter().any(|r| r.target_name == "BaseService" && r.kind == EdgeKind::Inherits),
            "Expected Inherits edge to BaseService; refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.iter().any(|r| r.target_name == "Serializable" && r.kind == EdgeKind::Implements),
            "Expected Implements edge to Serializable"
        );
        assert!(
            r.iter().any(|r| r.target_name == "Cloneable" && r.kind == EdgeKind::Implements),
            "Expected Implements edge to Cloneable"
        );
    }

    // -----------------------------------------------------------------------
    // 9. Interface extends interface → Implements edges
    // -----------------------------------------------------------------------
    #[test]
    fn interface_extends_interface() {
        let src = r#"
public interface ExtendedRepo extends Repository, ReadOnly {}
"#;
        let r = refs(src);

        assert!(
            r.iter().any(|r| r.target_name == "Repository" && r.kind == EdgeKind::Implements),
            "Expected Implements for Repository"
        );
        assert!(
            r.iter().any(|r| r.target_name == "ReadOnly" && r.kind == EdgeKind::Implements),
            "Expected Implements for ReadOnly"
        );
    }

    // -----------------------------------------------------------------------
    // 10. Visibility extraction
    // -----------------------------------------------------------------------
    #[test]
    fn visibility_extraction() {
        let src = r#"
class Example {
    public int pub;
    private int priv;
    protected int prot;
    int packagePrivate;
}
"#;
        let symbols = sym(src);

        let pub_field = symbols.iter().find(|s| s.name == "pub").unwrap();
        assert_eq!(pub_field.visibility, Some(Visibility::Public));

        let priv_field = symbols.iter().find(|s| s.name == "priv").unwrap();
        assert_eq!(priv_field.visibility, Some(Visibility::Private));

        let prot_field = symbols.iter().find(|s| s.name == "prot").unwrap();
        assert_eq!(prot_field.visibility, Some(Visibility::Protected));

        let pkg_field = symbols.iter().find(|s| s.name == "packagePrivate").unwrap();
        assert_eq!(pkg_field.visibility, None, "package-private should be None");
    }

    // -----------------------------------------------------------------------
    // 11. object_creation_expression → Instantiates edge
    // -----------------------------------------------------------------------
    #[test]
    fn instantiation_edges() {
        let src = r#"
class Factory {
    void create() {
        Foo f = new Foo();
        List<Bar> bars = new ArrayList<>();
    }
}
"#;
        let r = refs(src);
        let instantiations: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Instantiates).collect();
        let names: Vec<&str> = instantiations.iter().map(|r| r.target_name.as_str()).collect();

        assert!(names.contains(&"Foo"),       "Missing Foo; got: {names:?}");
        assert!(names.contains(&"ArrayList"), "Missing ArrayList; got: {names:?}");
    }

    // -----------------------------------------------------------------------
    // 12. Constructor extraction
    // -----------------------------------------------------------------------
    #[test]
    fn constructor_extracted() {
        let src = r#"
class Svc {
    public Svc(String name, int port) {}
}
"#;
        let symbols = sym(src);
        let ctor = symbols.iter().find(|s| s.kind == SymbolKind::Constructor).unwrap();
        assert_eq!(ctor.name, "Svc");
        assert_eq!(ctor.visibility, Some(Visibility::Public));
        let sig = ctor.signature.as_ref().unwrap();
        assert!(sig.contains("String"), "signature: {sig}");
        assert!(sig.contains("int"),    "signature: {sig}");
    }

    // -----------------------------------------------------------------------
    // 13. Does not panic on malformed source
    // -----------------------------------------------------------------------
    #[test]
    fn does_not_panic_on_malformed_source() {
        let src = "public class { broken !!! @@@ ###";
        let _ = extract::extract(src); // must not panic
    }

    // -----------------------------------------------------------------------
    // 14. Annotation type treated as interface
    // -----------------------------------------------------------------------
    #[test]
    fn annotation_type_as_interface() {
        let src = r#"
package com.example;

public @interface MyAnnotation {
    String value() default "";
}
"#;
        let symbols = sym(src);
        let ann = symbols.iter().find(|s| s.name == "MyAnnotation").unwrap();
        assert_eq!(ann.kind, SymbolKind::Interface);
        assert_eq!(ann.qualified_name, "com.example.MyAnnotation");
    }

    // -----------------------------------------------------------------------
    // instanceof narrowing
    // -----------------------------------------------------------------------

    #[test]
    fn instanceof_emits_type_ref() {
        let src = r#"
package com.example;
public class AuthService {
    public void check(Object user) {
        if (user instanceof Admin) {
            System.out.println("admin");
        }
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "refs: {r:?}"
        );
    }

    #[test]
    fn instanceof_pattern_variable_emits_variable_and_type_ref() {
        let src = r#"
package com.example;
public class AuthService {
    public void check(Object user) {
        if (user instanceof Admin admin) {
            admin.doStuff();
        }
    }
}
"#;
        let s = sym(src);
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "expected TypeRef to Admin, refs: {r:?}"
        );
        assert!(
            s.iter().any(|s| s.name == "admin" && s.kind == SymbolKind::Variable),
            "expected Variable symbol 'admin', symbols: {s:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Lambda extraction
    // -----------------------------------------------------------------------

    #[test]
    fn calls_inside_lambda_are_extracted() {
        let src = r#"
class Service {
    void run() {
        users.stream().map(u -> u.getName()).collect(Collectors.toList());
    }
}
"#;
        let r = refs(src);
        let calls: Vec<&str> = r.iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"getName"), "Missing 'getName' inside lambda: {calls:?}");
        assert!(calls.contains(&"map"),     "Missing 'map': {calls:?}");
    }

    #[test]
    fn lambda_parameter_emitted_as_variable_symbol() {
        let src = r#"
class Service {
    void run() {
        users.stream().map(u -> u.getName()).collect(Collectors.toList());
    }
}
"#;
        let s = sym(src);
        assert!(
            s.iter().any(|s| s.name == "u" && s.kind == SymbolKind::Variable),
            "expected Variable symbol 'u', symbols: {:?}",
            s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // record_declaration (Java 16+)
    // -----------------------------------------------------------------------

    #[test]
    fn record_declaration_extracted_as_class() {
        let src = r#"
package com.example;

public record Point(int x, int y) {}
"#;
        let s = sym(src);
        let rec = s.iter().find(|s| s.name == "Point");
        assert!(rec.is_some(), "expected Point record symbol");
        assert_eq!(rec.unwrap().kind, SymbolKind::Class, "record should map to Class");
        assert_eq!(rec.unwrap().qualified_name, "com.example.Point");
    }

    #[test]
    fn record_with_implements_emits_implements_edge() {
        let src = r#"
package com.example;

public record Named(String name) implements Comparable<Named> {}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Comparable" && r.kind == EdgeKind::Implements),
            "expected Implements edge for Comparable from record, refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // catch_clause
    // -----------------------------------------------------------------------

    #[test]
    fn catch_clause_emits_type_ref_and_variable() {
        let src = r#"
class Service {
    void run() {
        try {
            riskyCall();
        } catch (IOException e) {
            e.getMessage();
        }
    }
}
"#;
        let r = refs(src);
        let s = sym(src);
        assert!(
            r.iter().any(|r| r.target_name == "IOException" && r.kind == EdgeKind::TypeRef),
            "expected TypeRef to IOException from catch, refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            s.iter().any(|s| s.name == "e" && s.kind == SymbolKind::Variable),
            "expected Variable 'e' from catch clause"
        );
    }

    // -----------------------------------------------------------------------
    // cast_expression
    // -----------------------------------------------------------------------

    #[test]
    fn cast_expression_emits_type_ref() {
        let src = r#"
class Service {
    void run(Object obj) {
        Admin admin = (Admin) obj;
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
            "expected TypeRef to Admin from cast, refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // method_reference
    // -----------------------------------------------------------------------

    #[test]
    fn method_reference_emits_calls_edge() {
        let src = r#"
class Service {
    void run() {
        users.stream().map(User::getName).collect(Collectors.toList());
    }
}
"#;
        let r = refs(src);
        let calls: Vec<&str> = r.iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            calls.contains(&"getName"),
            "expected Calls edge for getName from method_reference, got: {calls:?}"
        );
    }

    // -----------------------------------------------------------------------
    // enhanced_for_statement
    // -----------------------------------------------------------------------

    #[test]
    fn enhanced_for_emits_variable_and_type_ref() {
        let src = r#"
class Service {
    void run(List<User> users) {
        for (User user : users) {
            user.activate();
        }
    }
}
"#;
        let r = refs(src);
        let s = sym(src);
        assert!(
            r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
            "expected TypeRef to User from enhanced-for, refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
        assert!(
            s.iter().any(|s| s.name == "user" && s.kind == SymbolKind::Variable),
            "expected Variable 'user' from enhanced-for, symbols: {:?}",
            s.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // class_literal
    // -----------------------------------------------------------------------

    #[test]
    fn class_literal_emits_type_ref() {
        let src = r#"
class Service {
    void run() {
        Class<?> cls = User.class;
    }
}
"#;
        let r = refs(src);
        assert!(
            r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
            "expected TypeRef to User from class literal, refs: {:?}",
            r.iter().map(|r| (&r.target_name, r.kind)).collect::<Vec<_>>()
        );
    }
