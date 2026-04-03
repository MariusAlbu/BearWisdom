// =============================================================================
// typescript/coverage_tests.rs
//
// One test per node kind declared in TypeScriptPlugin::symbol_node_kinds() and
// ref_node_kinds(). Each test parses a minimal snippet and asserts the expected
// Symbol or Ref is produced.
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn coverage_class_declaration() {
    let r = extract::extract("class Foo {}", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "Foo"),
        "class_declaration should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_abstract_class_declaration() {
    let r = extract::extract("abstract class Shape {}", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "Shape"),
        "abstract_class_declaration should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_interface_declaration() {
    let r = extract::extract("interface IRepo {}", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Interface && s.name == "IRepo"),
        "interface_declaration should produce Interface symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_function_declaration() {
    let r = extract::extract("function doWork(): void {}", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "doWork"),
        "function_declaration should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_generator_function_declaration() {
    let r = extract::extract("function* gen(): Generator<number> { yield 1; }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "gen"),
        "generator_function_declaration should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_definition() {
    let r = extract::extract("class Svc { handle(): void {} }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "handle"),
        "method_definition should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_abstract_method_signature() {
    let r = extract::extract("abstract class Base { abstract run(): void; }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "run"),
        "abstract_method_signature should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_signature() {
    let r = extract::extract("interface IRepo { findOne(id: number): User; }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "findOne"),
        "method_signature should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_public_field_definition() {
    let r = extract::extract("class Svc { public name: string = ''; }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "name"),
        "public_field_definition should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_property_signature() {
    let r = extract::extract("interface Config { timeout: number; }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "timeout"),
        "property_signature should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_field_definition() {
    // Private field (no accessibility modifier) — standard field_definition.
    let r = extract::extract("class Svc { count = 0; }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "count"),
        "field_definition should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_alias_declaration() {
    let r = extract::extract("type UserId = string;", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::TypeAlias && s.name == "UserId"),
        "type_alias_declaration should produce TypeAlias symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_enum_declaration() {
    let r = extract::extract("enum Status { Active, Inactive }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Enum && s.name == "Status"),
        "enum_declaration should produce Enum symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_enum_body() {
    // enum_body is the container of enum members; members should appear as EnumMember symbols.
    let r = extract::extract("enum Direction { Up, Down, Left, Right }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::EnumMember && s.name == "Up"),
        "enum_body should produce EnumMember symbols; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_lexical_declaration() {
    let r = extract::extract("const apiUrl: string = 'http://example.com';", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "apiUrl"),
        "lexical_declaration should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_variable_declaration() {
    // `var` produces variable_declaration (not lexical_declaration).
    let r = extract::extract("var legacyVar = 42;", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "legacyVar"),
        "variable_declaration should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_internal_module() {
    let r = extract::extract("namespace MyNS { export const x = 1; }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace && s.name == "MyNS"),
        "internal_module should produce Namespace symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_construct_signature() {
    // Interface with a construct signature: `new(name: string): Product`
    let r = extract::extract("interface Factory { new(name: string): Product; }", false);
    assert!(
        r.symbols
            .iter()
            .any(|s| (s.kind == SymbolKind::Constructor || s.kind == SymbolKind::Method)
                && s.name == "new"),
        "construct_signature should produce Constructor or Method symbol named 'new'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_call_signature() {
    // Interface with a call signature: `(x: number): string`
    let r = extract::extract("interface Callable { (x: number): string; }", false);
    assert!(
        r.symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Method && s.name == "call"),
        "call_signature should produce Method symbol named 'call'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_index_signature() {
    let r = extract::extract("interface Lookup { [key: string]: User; }", false);
    assert!(
        r.symbols
            .iter()
            .any(|s| s.kind == SymbolKind::Property && s.name.contains("key")),
        "index_signature should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn coverage_call_expression() {
    let r = extract::extract("function run() { fetchData(); }", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "fetchData"),
        "call_expression should produce Calls ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_new_expression() {
    let r = extract::extract("const x = new EventEmitter();", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Instantiates && r.target_name == "EventEmitter"),
        "new_expression should produce Instantiates ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_import_statement() {
    let r = extract::extract(r#"import { UserService } from "./user";"#, false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "UserService"),
        "import_statement should produce TypeRef ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_jsx_self_closing_element() {
    // Use TSX grammar for JSX parsing.
    let r = extract::extract("function App() { return <Button />; }", true);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "Button"),
        "jsx_self_closing_element should produce Calls ref for PascalCase components; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_jsx_opening_element() {
    let r = extract::extract("function App() { return <Modal>content</Modal>; }", true);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "Modal"),
        "jsx_opening_element should produce Calls ref for PascalCase components; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_extends_clause() {
    let r = extract::extract("class Dog extends Animal {}", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Inherits && r.target_name == "Animal"),
        "extends_clause should produce Inherits ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_implements_clause() {
    let r = extract::extract("class UserRepo implements IRepository {}", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Implements && r.target_name == "IRepository"),
        "implements_clause should produce Implements ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_annotation() {
    // Variable with an explicit type annotation: `const x: UserService = null`
    // Should emit a TypeRef to UserService from x.
    let r = extract::extract("const x: UserService = null as any;", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "UserService"),
        "type_annotation should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_identifier() {
    // type_identifier appears as a reference within a type alias body.
    // `type Alias = TargetType` → TypeRef to TargetType.
    let r = extract::extract("type Alias = TargetType;", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "TargetType"),
        "type_identifier should produce TypeRef (via type alias value); got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_as_expression() {
    let r = extract::extract("const admin = user as Admin;", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "Admin"),
        "as_expression should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_satisfies_expression() {
    let r = extract::extract("const cfg = { debug: true } satisfies AppConfig;", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "AppConfig"),
        "satisfies_expression should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_binary_expression_instanceof() {
    let r = extract::extract(
        "function check(x: unknown) { if (x instanceof AdminUser) {} }",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "AdminUser"),
        "binary_expression instanceof should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_tagged_template_expression() {
    let r = extract::extract("function run() { const q = sql`SELECT 1`; }", false);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "sql"),
        "tagged_template_expression should produce Calls ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}
