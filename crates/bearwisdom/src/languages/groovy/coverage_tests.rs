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
