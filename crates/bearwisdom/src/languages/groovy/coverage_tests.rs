// =============================================================================
// groovy/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
//
// Grammar node kinds (confirmed by CST probe):
//   class_declaration  — class body
//   method_declaration — typed method inside class body
//   function_definition — top-level `def fn(...)`
//   package_declaration — package statement
//   import_declaration  — import statement
//   method_invocation   — call expression
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `class_declaration`  →  Class
#[test]
fn symbol_class_definition() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Foo" && s.kind == SymbolKind::Class),
        "expected Class Foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Class methods in packaged Groovy file get qualified_name and scope_path set.
#[test]
fn groovy_packaged_class_methods_are_qualified() {
    let src = "package org.codenarc.rule\n\nabstract class AbstractRuleTestCase<T extends Rule> extends AbstractTestCase {\n    void assertSingleViolation(String code) {}\n    void testFoo() {\n        assertSingleViolation('code')\n    }\n}";
    let r = extract(src);
    let cls = r.symbols.iter().find(|s| s.name == "AbstractRuleTestCase").expect("missing class");
    assert_eq!(cls.qualified_name, "org.codenarc.rule.AbstractRuleTestCase");
    let method = r.symbols.iter().find(|s| s.name == "assertSingleViolation").expect("missing method");
    assert_eq!(method.scope_path.as_deref(), Some("org.codenarc.rule.AbstractRuleTestCase"));
    assert_eq!(method.qualified_name, "org.codenarc.rule.AbstractRuleTestCase.assertSingleViolation");
}

/// Probe exact AbstractAstVisitorRuleTest syntax from groovy-codenarc.
#[test]
fn groovy_concrete_subclass_with_generic_parent() {
    let src = "package org.codenarc.rule\n\nimport static org.codenarc.test.TestUtil.shouldFailWithMessageContaining\n\nclass AbstractAstVisitorRuleTest extends AbstractRuleTestCase<AbstractAstVisitorRule> {\n    void testApplyTo() {\n        assertSingleViolation('code')\n    }\n}";
    let r = extract(src);
    eprintln!("has_errors={}, symbols={:?}", r.has_errors, r.symbols.iter().map(|s| (&s.name, s.kind, s.scope_path.as_deref())).collect::<Vec<_>>());
    assert!(
        r.symbols.iter().any(|s| s.name == "AbstractAstVisitorRuleTest"),
        "expected class symbol; got {:?}",
        r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `function_definition` (top-level `def`)  →  Function
#[test]
fn symbol_function_definition_top_level() {
    let r = extract("def greet(name) {\n    println(name)\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "greet" && s.kind == SymbolKind::Function),
        "expected Function greet; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `method_declaration` inside class  →  Method
#[test]
fn symbol_function_definition_method() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "bar" && s.kind == SymbolKind::Method),
        "expected Method bar; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: typed `method_declaration`  →  Method
#[test]
fn symbol_function_declaration() {
    let r = extract("class Calc {\n    int add(int a, int b) { return a + b }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "add" && s.kind == SymbolKind::Method),
        "expected Method add; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `package_declaration`  →  Namespace
#[test]
fn symbol_groovy_package() {
    let r = extract("package com.example.app\n\nclass Hello {}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace),
        "expected Namespace from package_declaration; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `method_invocation`  →  Calls edge
#[test]
fn ref_function_call() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "baz" && rf.kind == EdgeKind::Calls),
        "expected Calls baz; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: top-level `method_invocation` (like println)
#[test]
fn ref_juxt_function_call() {
    let r = extract("def run() {\n    println(\"hello\")\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "println" && rf.kind == EdgeKind::Calls),
        "expected Calls println; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `import_declaration`  →  Imports edge
#[test]
fn ref_groovy_import() {
    let r = extract("import groovy.json.JsonSlurper\n\nclass Foo {}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from import_declaration; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional symbol node kinds — declaration (Field / Variable), nested class,
// package name format
// ---------------------------------------------------------------------------

/// `field_declaration` inside a class body → Field symbol
#[test]
fn symbol_field_declaration() {
    let r = extract("class Foo {\n    String name = \"hello\"\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "name" && s.kind == SymbolKind::Field),
        "expected Field name; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// `package_declaration` → Namespace with fully qualified dotted name
#[test]
fn symbol_groovy_package_name_format() {
    let r = extract("package com.example.app\n\nclass Hello {}");
    let ns: Vec<_> = r
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Namespace)
        .collect();
    assert!(
        !ns.is_empty(),
        "expected Namespace symbol from package; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        ns[0].name.contains('.'),
        "Namespace name should be dotted (com.example.app); got '{}'",
        ns[0].name
    );
}

/// Nested class → Class symbol with parent_index pointing to outer class
#[test]
fn symbol_nested_class() {
    let r = extract("class Outer {\n    class Inner {}\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Outer" && s.kind == SymbolKind::Class),
        "expected Class Outer; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Inner" && s.kind == SymbolKind::Class),
        "expected Class Inner; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    let inner = r.symbols.iter().find(|s| s.name == "Inner").unwrap();
    assert!(
        inner.parent_index.is_some(),
        "nested class Inner should have parent_index set; got {:?}",
        inner.parent_index
    );
}

// ---------------------------------------------------------------------------
// Additional ref node kinds — wildcard import, chained method call
// ---------------------------------------------------------------------------

/// Wildcard import → Imports edge (module name without trailing `.*`)
#[test]
fn ref_groovy_wildcard_import() {
    let r = extract("import groovy.json.*\n\nclass Foo {}");
    assert!(
        r.refs
            .iter()
            .any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("groovy.json")),
        "expected Imports from wildcard import; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Multiple method calls in a method body → multiple Calls edges
#[test]
fn ref_multiple_calls_in_method() {
    let r = extract("class Foo {\n    def run() {\n        bar()\n        baz()\n    }\n}");
    let calls: Vec<_> = r.refs.iter().filter(|rf| rf.kind == EdgeKind::Calls).collect();
    assert!(
        calls.len() >= 2,
        "expected >= 2 Calls edges; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// `class extends Super` → Inherits ref
#[test]
fn ref_class_extends_produces_inherits() {
    let r = extract("class Dog extends Animal {}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "Animal"),
        "expected Inherits(Animal) from extends clause; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// `class implements Interface` → Implements ref
#[test]
fn ref_class_implements_produces_implements() {
    let r = extract("class Foo implements IBar {}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements && rf.target_name == "IBar"),
        "expected Implements(IBar) from implements clause; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// abstract class with generic bound `<T extends X>` is extracted as Class
#[test]
fn symbol_abstract_class_with_generic_bound() {
    let r = extract("package org.codenarc.rule\n\nabstract class AbstractRuleTestCase<T extends Rule> extends AbstractTestCase {\n    protected void assertSingleViolation(String code) {}\n}");
    eprintln!("has_errors={}, symbols={:?}", r.has_errors, r.symbols.iter().map(|s| (&s.name, s.kind, &s.qualified_name)).collect::<Vec<_>>());
    assert!(
        r.symbols.iter().any(|s| s.name == "AbstractRuleTestCase" && s.kind == SymbolKind::Class),
        "expected Class AbstractRuleTestCase; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    let method = r.symbols.iter().find(|s| s.name == "assertSingleViolation");
    assert!(method.is_some(), "expected method assertSingleViolation");
    let method = method.unwrap();
    assert_eq!(method.qualified_name, "org.codenarc.rule.AbstractRuleTestCase.assertSingleViolation");
}

/// annotated class (with @Annotation before class keyword) is extracted as Class
#[test]
fn symbol_annotated_class_is_extracted() {
    let r = extract("package org.codenarc.rule\n\n@SuppressWarnings('DuplicateLiteral')\nabstract class AbstractRuleTestCase<T extends Rule> extends AbstractTestCase {\n    protected void assertSingleViolation(String code) {}\n}");
    eprintln!("annotated class: has_errors={}, symbols={:?}", r.has_errors, r.symbols.iter().map(|s| (&s.name, s.kind, &s.qualified_name)).collect::<Vec<_>>());
    assert!(
        r.symbols.iter().any(|s| s.name == "AbstractRuleTestCase" && s.kind == SymbolKind::Class),
        "expected Class AbstractRuleTestCase with @SuppressWarnings; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// class with static final field using angle bracket literal causes parse error check
#[test]
fn symbol_class_with_angle_bracket_literal() {
    // '<init>' in a field causes Groovy grammar confusion with generic types
    let src = "package org.codenarc.rule\n\n@SuppressWarnings('DuplicateLiteral')\nabstract class AbstractRuleTestCase<T extends Rule> extends AbstractTestCase {\n    protected static final CONSTRUCTOR_METHOD_NAME = '<init>'\n    protected void assertSingleViolation(String code) {}\n}";
    let r = extract(src);
    eprintln!("angle bracket literal: has_errors={}", r.has_errors);
    for s in &r.symbols {
        eprintln!("  {:?} name={} qname={} scope={:?}", s.kind, s.name, s.qualified_name, s.scope_path);
    }
    assert!(
        r.symbols.iter().any(|s| s.name == "AbstractRuleTestCase" && s.kind == SymbolKind::Class),
        "expected Class AbstractRuleTestCase with '<init>' literal; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    let method = r.symbols.iter().find(|s| s.name == "assertSingleViolation");
    assert!(method.is_some(), "expected method assertSingleViolation");
    let m = method.unwrap();
    assert_eq!(m.qualified_name, "org.codenarc.rule.AbstractRuleTestCase.assertSingleViolation",
        "method qname should be fully qualified");
    assert_eq!(m.scope_path.as_deref(), Some("org.codenarc.rule.AbstractRuleTestCase"),
        "method scope_path should point to class");
}

/// Method with typed return type → Method symbol still emitted
#[test]
fn symbol_method_with_return_type_produces_method() {
    let r = extract("class Service {\n    List<String> getNames() {\n        return []\n    }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "getNames" && s.kind == SymbolKind::Method),
        "expected Method getNames; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Test reading the actual AbstractRuleTestCase.groovy file from disk (integration-style)
#[test]
#[ignore = "requires test project on disk; run manually"]
fn symbol_actual_abstract_rule_test_case_from_disk() {
    let path = "F:/Work/Projects/TestProjects/groovy-codenarc/src/main/groovy/org/codenarc/rule/AbstractRuleTestCase.groovy";
    let src = std::fs::read_to_string(path).expect("file not found");
    let r = extract(&src);
    eprintln!("actual file from disk: has_errors={}", r.has_errors);
    for s in &r.symbols {
        eprintln!("  {:?} name={} qname={}", s.kind, s.name, s.qualified_name);
    }
    assert!(
        r.symbols.iter().any(|s| s.name == "AbstractRuleTestCase" && s.kind == SymbolKind::Class),
        "expected Class AbstractRuleTestCase from file; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Test with the actual AbstractRuleTestCase.groovy content (first 100 lines)
#[test]
fn symbol_actual_abstract_rule_test_case() {
    // Minimal reproduction: package + import + annotation + abstract class with '<init>'
    let src = r#"package org.codenarc.rule

import org.codenarc.test.AbstractTestCase
import org.junit.jupiter.api.Test

import java.util.regex.Pattern

@SuppressWarnings('DuplicateLiteral')
abstract class AbstractRuleTestCase<T extends Rule> extends AbstractTestCase {

    protected static final CONSTRUCTOR_METHOD_NAME = '<init>'
    protected T rule

    @Test
    void testThatUnrelatedCodeHasNoViolations() {
        final SOURCE = 'class MyClass { }'
        assertNoViolations(SOURCE)
    }

    protected void assertSingleViolation(String code) {}
    protected void assertViolations(String code, Map[] args) {}
}"#;
    let r = extract(src);
    eprintln!("actual file: has_errors={}", r.has_errors);
    for s in &r.symbols {
        eprintln!("  {:?} name={} qname={} scope={:?}", s.kind, s.name, s.qualified_name, s.scope_path);
    }
    assert!(
        r.symbols.iter().any(|s| s.name == "AbstractRuleTestCase" && s.kind == SymbolKind::Class),
        "expected Class AbstractRuleTestCase; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Round-2 fixes: static methods + import static
// ---------------------------------------------------------------------------

/// `static String method(...)` inside a class → Method symbol is emitted
/// (tree-sitter-groovy sometimes misses static methods as method_declaration).
#[test]
fn symbol_static_string_method_is_extracted() {
    let r = extract(
        "class TestUtil {\n\
         \n\
             static String shouldFail(Class expectedExceptionClass, Closure code) {\n\
         \n\
                 return null\n\
         \n\
             }\n\
         \n\
         }",
    );
    eprintln!(
        "static_string_method: has_errors={}, symbols={:?}",
        r.has_errors,
        r.symbols
            .iter()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
    assert!(
        r.symbols
            .iter()
            .any(|s| s.name == "shouldFail" && s.kind == SymbolKind::Method),
        "expected Method shouldFail from `static String shouldFail(...)`; got {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
}

/// `static void method(...)` inside a class → Method symbol emitted with correct qname
#[test]
fn symbol_static_void_method_is_extracted_with_qname() {
    let r = extract(
        "package org.codenarc.test\n\
         class TestUtil {\n\
         \n\
             static void assertContainsAll(String text, Collection strings) {\n\
         \n\
                 strings.each { assert text.contains(it.toString()) }\n\
         \n\
             }\n\
         \n\
         }",
    );
    let method = r.symbols.iter().find(|s| s.name == "assertContainsAll");
    assert!(
        method.is_some(),
        "expected Method assertContainsAll; got {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
    let m = method.unwrap();
    assert_eq!(
        m.qualified_name, "org.codenarc.test.TestUtil.assertContainsAll",
        "static method qname should include package and class"
    );
    assert_eq!(
        m.scope_path.as_deref(),
        Some("org.codenarc.test.TestUtil"),
        "static method scope_path should point to class"
    );
}

/// `private static Type movedTo(...)` → Method symbol emitted
#[test]
fn symbol_private_static_method_is_extracted() {
    let r = extract(
        "class MovedRules {\n\
         \n\
             private static MovedToRuleSet movedTo(String ruleSetName) {\n\
         \n\
                 return new MovedToRuleSet(ruleSetName)\n\
         \n\
             }\n\
         \n\
         }",
    );
    assert!(
        r.symbols
            .iter()
            .any(|s| s.name == "movedTo" && s.kind == SymbolKind::Method),
        "expected Method movedTo from `private static MovedToRuleSet movedTo(...)`; got {:?}",
        r.symbols
            .iter()
            .map(|s| (&s.name, s.kind))
            .collect::<Vec<_>>()
    );
}

/// `import static pkg.Class.member` → Imports ref with target_name = "member" and module = full path
#[test]
fn ref_import_static_produces_member_import() {
    let r = extract(
        "import static org.codenarc.test.TestUtil.shouldFail\n\
         class Foo {}",
    );
    let import_ref = r
        .refs
        .iter()
        .find(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "shouldFail");
    assert!(
        import_ref.is_some(),
        "expected Imports ref with target_name='shouldFail'; got {:?}",
        r.refs
            .iter()
            .map(|rf| (&rf.target_name, rf.kind, rf.module.as_deref()))
            .collect::<Vec<_>>()
    );
    let ir = import_ref.unwrap();
    assert_eq!(
        ir.module.as_deref(),
        Some("org.codenarc.test.TestUtil.shouldFail"),
        "static import module should be the full qualified path"
    );
}

/// `import static pkg.Class.method` bare call resolves via exact-import lookup
/// (integration: extract + inherit-aware Java resolver step 3)
#[test]
fn ref_import_static_not_polluted_as_module() {
    // The old bug: `import static org.example.Util.foo` would produce
    // target_name = "static" (entire first word after "import").
    // After the fix, target_name must be "foo", not "static".
    let r = extract(
        "import static org.example.Util.foo\n\
         class MyTest {}",
    );
    // Must NOT have a ref with target_name = "static"
    assert!(
        !r.refs
            .iter()
            .any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "static"),
        "import static must not produce target_name='static'; got {:?}",
        r.refs
            .iter()
            .filter(|rf| rf.kind == EdgeKind::Imports)
            .map(|rf| (&rf.target_name, rf.module.as_deref()))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Hierarchy ref enrichment from imports
// ---------------------------------------------------------------------------

/// Inherits ref gets module annotated from matching import declaration.
///
/// `import spock.lang.Specification` + `class MySpec extends Specification`
/// → the Inherits ref for Specification has module="spock.lang.Specification".
#[test]
fn ref_inherits_gets_module_from_import() {
    let src = "import spock.lang.Specification\n\
               class MySpec extends Specification {\n\
                   def 'a feature'() { expect: true }\n\
               }";
    let r = extract(src);
    let inh = r.refs.iter().find(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "Specification");
    assert!(
        inh.is_some(),
        "expected Inherits ref for Specification; refs={:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        inh.unwrap().module.as_deref(),
        Some("spock.lang.Specification"),
        "Inherits ref must carry the FQN from the import"
    );
}

/// Implements ref gets module annotated from matching import declaration.
#[test]
fn ref_implements_gets_module_from_import() {
    let src = "import java.io.Serializable\n\
               class MyClass implements Serializable {}";
    let r = extract(src);
    let imp = r.refs.iter().find(|rf| rf.kind == EdgeKind::Implements && rf.target_name == "Serializable");
    assert!(
        imp.is_some(),
        "expected Implements ref for Serializable; refs={:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        imp.unwrap().module.as_deref(),
        Some("java.io.Serializable"),
        "Implements ref must carry the FQN from the import"
    );
}

/// Inherits ref without a matching import leaves module=None.
#[test]
fn ref_inherits_without_import_leaves_module_none() {
    let src = "class Child extends Parent {}";
    let r = extract(src);
    let inh = r.refs.iter().find(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "Parent");
    assert!(inh.is_some(), "expected Inherits ref for Parent");
    assert_eq!(
        inh.unwrap().module,
        None,
        "Inherits without matching import must leave module=None"
    );
}

/// symbol_node_kind: `interface_declaration` → Class symbol is emitted.
#[test]
fn symbol_interface_declaration() {
    let src = "interface Serializable {}";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "Serializable" && s.kind == SymbolKind::Class),
        "expected Class symbol for interface Serializable; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Nested interface inside a class body is extracted as a Class symbol.
#[test]
fn symbol_nested_interface_in_class() {
    let src = "class Outer {\n    interface Inner {}\n}";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "Inner" && s.kind == SymbolKind::Class),
        "expected Class symbol for nested interface Inner; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Interface with `extends` emits an Inherits edge to each parent interface.
#[test]
fn ref_interface_extends_emits_inherits() {
    let src = "interface Child extends Parent {}";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "Parent"),
        "expected Inherits ref to Parent from interface; refs={:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 2-space indent extraction + inner-class attribution
// ---------------------------------------------------------------------------

/// Methods at 2-space indent (class at col 0) are extracted with correct scope_path.
///
/// The tree-sitter-groovy grammar sometimes misses `private void` / `static boolean`
/// declarations that the supplemental scanner must recover.
#[test]
fn symbol_two_space_private_method_is_extracted() {
    let src = "package com.example\n\
               class Utils {\n\
               \n\
               \x20\x20static boolean isAndroid(Project p) {\n\
               \x20\x20\x20\x20return false\n\
               \x20\x20}\n\
               \n\
               \x20\x20private void checkReady() {\n\
               \x20\x20\x20\x20// no-op\n\
               \x20\x20}\n\
               }";
    let r = extract(src);
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        r.symbols.iter().any(|s| s.name == "isAndroid" && s.kind == SymbolKind::Method),
        "expected Method isAndroid at 2-space indent; symbols={:?}", names
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "checkReady" && s.kind == SymbolKind::Method),
        "expected Method checkReady at 2-space indent; symbols={:?}", names
    );
    // scope_path must point to the outer class, not an inner class
    for sym in r.symbols.iter().filter(|s| s.name == "isAndroid" || s.name == "checkReady") {
        assert_eq!(
            sym.scope_path.as_deref(),
            Some("com.example.Utils"),
            "{} scope_path should be com.example.Utils; got {:?}", sym.name, sym.scope_path
        );
    }
}

/// Inner class methods are extracted with their correct grammar-assigned scope_path.
/// The grammar correctly attributes innerMethod to Inner; outerMethod to Outer.
#[test]
fn symbol_inner_class_methods_not_attributed_to_outer() {
    let src = "package com.example\n\
               class Outer {\n\
               \n\
               \x20\x20public class Inner {\n\
               \x20\x20\x20\x20void innerMethod() {}\n\
               \x20\x20}\n\
               \n\
               \x20\x20void outerMethod() {}\n\
               }";
    let r = extract(src);
    // outerMethod must be scoped to Outer, not Inner
    let outer_m = r.symbols.iter().find(|s| s.name == "outerMethod");
    assert!(outer_m.is_some(), "expected outerMethod; symbols={:?}",
        r.symbols.iter().map(|s| (&s.name, s.scope_path.as_deref())).collect::<Vec<_>>());
    assert_eq!(
        outer_m.unwrap().scope_path.as_deref(),
        Some("com.example.Outer"),
        "outerMethod must be scoped to Outer"
    );
    // innerMethod must be scoped to Inner (grammar handles it)
    let inner_m = r.symbols.iter().find(|s| s.name == "innerMethod");
    assert!(inner_m.is_some(), "expected innerMethod from grammar; symbols={:?}",
        r.symbols.iter().map(|s| (&s.name, s.scope_path.as_deref())).collect::<Vec<_>>());
    assert_eq!(
        inner_m.unwrap().scope_path.as_deref(),
        Some("com.example.Inner"),
        "innerMethod must be scoped to Inner, not Outer"
    );
}

// ---------------------------------------------------------------------------
// Call extraction: all method_invocation receivers produce bare name calls
// ---------------------------------------------------------------------------

/// Both static-receiver and instance-receiver calls produce bare `name` refs.
/// `Utils.doSomething(p)` → Calls ref with target_name = "doSomething".
/// The resolver resolves via groovy_bare_name (which accepts .groovy paths)
/// once the symbol is indexed with the correct qname.
#[test]
fn ref_static_class_call_emits_bare_method_name() {
    let src = "class Foo {\n\
               \x20\x20def bar(p) {\n\
               \x20\x20\x20\x20Utils.doSomething(p)\n\
               \x20\x20}\n\
               }";
    let r = extract(src);
    let call = r.refs.iter().find(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "doSomething");
    assert!(
        call.is_some(),
        "expected Calls ref with target_name='doSomething'; refs={:?}",
        r.refs.iter().filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

/// Instance-method calls on variable receivers produce bare method name refs.
#[test]
fn ref_instance_call_on_lowercase_receiver_stays_bare() {
    let src = "class Foo {\n\
               \x20\x20def bar(project) {\n\
               \x20\x20\x20\x20project.afterEvaluate()\n\
               \x20\x20}\n\
               }";
    let r = extract(src);
    // "afterEvaluate" must appear as a bare call — the resolver picks it up
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "afterEvaluate"),
        "expected bare Calls ref for afterEvaluate; refs={:?}",
        r.refs.iter().filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

/// 4-space indented class where grammar emits some methods (at col 4) but misses
/// `private void method(` with continuation args on the next line.
/// The supplemental scanner must detect member_indent=4 from grammar methods
/// and recover the multi-line declaration.
#[test]
fn symbol_four_space_multiline_method_recovered() {
    let src = "package com.example\n\
               \n\
               class ProtobufPlugin {\n\
               \n\
               \x20\x20\x20\x20void apply(Project project) {}\n\
               \n\
               \x20\x20\x20\x20private void doApply() {}\n\
               \n\
               \x20\x20\x20\x20private void addTasksForSourceSet(\n\
               \x20\x20\x20\x20\x20\x20\x20\x20SourceSet sourceSet, Configuration config) {}\n\
               }";
    let r = extract(src);
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        r.symbols.iter().any(|s| s.name == "addTasksForSourceSet" && s.kind == SymbolKind::Method),
        "expected Method addTasksForSourceSet recovered by supplemental scanner; symbols={:?}", names
    );
}

/// Regression guard: the actual ProtobufPlugin.groovy file must have
/// addTasksForSourceSet and addTasksForVariant extracted as methods.
#[test]
fn symbol_protobuf_plugin_actual_file_methods_extracted() {
    let path = "F:/Work/Projects/TestProjects/groovy-gradle-plugin/src/main/groovy/com/google/protobuf/gradle/ProtobufPlugin.groovy";
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return, // skip if test project not present
    };
    let r = extract(&src);
    eprintln!("has_errors={}", r.has_errors);
    eprintln!("symbols:");
    for s in &r.symbols {
        eprintln!("  {:?} name={} col={} scope={:?}", s.kind, s.name, s.start_col, s.scope_path.as_deref());
    }
    assert!(
        r.symbols.iter().any(|s| s.name == "addTasksForSourceSet" && s.kind == SymbolKind::Method),
        "expected Method addTasksForSourceSet; has_errors={} symbols={:?}",
        r.has_errors,
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "addTasksForVariant" && s.kind == SymbolKind::Method),
        "expected Method addTasksForVariant; has_errors={} symbols={:?}",
        r.has_errors,
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Same as above but with `implements Plugin<Project>` generic clause which can
/// trigger grammar parse errors — scanner must still recover multiline methods.
#[test]
fn symbol_four_space_multiline_method_recovered_with_generic_implements() {
    let src = "package com.example\n\
               \n\
               @CompileStatic\n\
               class ProtobufPlugin implements Plugin<Project> {\n\
               \n\
               \x20\x20\x20\x20void apply(Project project) {}\n\
               \n\
               \x20\x20\x20\x20private void doApply() {}\n\
               \n\
               \x20\x20\x20\x20private void addTasksForSourceSet(\n\
               \x20\x20\x20\x20\x20\x20\x20\x20SourceSet sourceSet, Configuration config) {}\n\
               }";
    let r = extract(src);
    eprintln!("has_errors={}, symbols={:?}", r.has_errors,
        r.symbols.iter().map(|s| (&s.name, s.kind, s.start_col)).collect::<Vec<_>>());
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        r.symbols.iter().any(|s| s.name == "addTasksForSourceSet" && s.kind == SymbolKind::Method),
        "expected Method addTasksForSourceSet recovered; has_errors={} symbols={:?}", r.has_errors, names
    );
}
