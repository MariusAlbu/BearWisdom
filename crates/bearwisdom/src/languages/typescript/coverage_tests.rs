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
fn coverage_ts_rallly_exact_poll_provider_structure() {
    // Byte-exact copy of the structure in ts-rallly's poll-context.tsx
    // (names changed, types simplified, but the SYNTAX shape is preserved).
    let r = extract::extract(
        r#"
import React from 'react';
import { useTranslation } from 'react-i18next';

type PollContextValue = { poll: { id: string } };
const PollContext = React.createContext<PollContextValue | null>(null);

export const PollContextProvider: React.FunctionComponent<{
  poll: { id: string };
  children?: React.ReactNode;
}> = ({ poll, children }) => {
  const { t } = useTranslation();
  const contextValue = React.useMemo<PollContextValue>(
    () => ({ poll }),
    [poll, t],
  );
  return (
    <PollContext.Provider value={contextValue}>{children}</PollContext.Provider>
  );
};
"#,
        true,
    );
    let provider_calls: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls && r.target_name == "Provider")
        .collect();
    assert!(
        !provider_calls.is_empty(),
        "Provider Calls ref must emit for the ts-rallly poll-context.tsx structure; got refs: {:#?}",
        r.refs
            .iter()
            .map(|r| (r.kind, r.target_name.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn coverage_annotated_const_with_destructured_params_jsx() {
    // Exact shape used in ts-rallly's poll-context.tsx:
    //   export const Foo: React.FC<Props> = ({ a, b }) => { return <X.Provider ...> }
    let r = extract::extract(
        r#"
import React from 'react';
const PollContext = React.createContext(null);
export const PollContextProvider: React.FC<{ x: number }> = ({ x }) => {
    return <PollContext.Provider value={x}>x</PollContext.Provider>;
};
"#,
        true,
    );
    let provider_calls: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls && r.target_name == "Provider")
        .collect();
    assert!(
        !provider_calls.is_empty(),
        "annotated-const with destructured params must still emit Provider Calls; refs: {:?}",
        r.refs
            .iter()
            .map(|r| (r.kind, r.target_name.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn coverage_real_ts_rallly_options_context_pattern() {
    // Exact shape of ts-rallly's poll-context.tsx OptionsProvider.
    let r = extract::extract(
        r#"
import React from 'react';
type OptionsContextValue = {
    pollType: string;
    options: string[];
};
const OptionsContext = React.createContext<OptionsContextValue>({} as OptionsContextValue);
const OptionsProvider = (props: { children: React.ReactNode }) => {
    const options: OptionsContextValue = { pollType: "date", options: [] };
    return (
        <OptionsContext.Provider value={options}>
            {props.children}
        </OptionsContext.Provider>
    );
};
"#,
        true,
    );
    let provider_calls: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls && r.target_name == "Provider")
        .collect();
    assert!(
        !provider_calls.is_empty(),
        "expected Provider Calls ref; refs: {:?}",
        r.refs
            .iter()
            .map(|r| (r.kind, r.target_name.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn coverage_destructure_default_string_does_not_emit_typeref() {
    // ONLY the destructure-with-default-value pattern, no interface with
    // literal unions.
    let src = r#"
const { variant = 'default', size = 'sm' } = Astro.props;
"#;
    let r = extract::extract(src, false);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    eprintln!("destructure-default type_refs: {type_refs:?}");
    for literal in &["default", "sm"] {
        assert!(
            !type_refs.contains(literal),
            "destructure default value `'{literal}'` must not emit TypeRef"
        );
    }
}

#[test]
fn coverage_interface_union_only_does_not_emit_typeref() {
    // ONLY the interface literal union, no destructuring.
    let src = r#"
interface Props {
    variant?: 'default' | 'success';
}
"#;
    let r = extract::extract(src, false);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    eprintln!("interface-union type_refs: {type_refs:?}");
    for literal in &["default", "success"] {
        assert!(
            !type_refs.contains(literal),
            "union literal `'{literal}'` must not emit TypeRef"
        );
    }
}

#[test]
fn coverage_union_with_five_literals() {
    let src = "interface Props {\n  variant?: 'default' | 'success' | 'warning' | 'danger' | 'outline';\n}\n";
    let r = extract::extract(src, false);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    eprintln!("5-literal type_refs: {type_refs:?}");
    for literal in &["default", "success", "warning", "danger", "outline"] {
        assert!(
            !type_refs.contains(literal),
            "`{literal}` must not emit TypeRef"
        );
    }
}

#[test]
fn coverage_literal_type_union_does_not_emit_typeref() {
    // Astro frontmatter pattern — a Props interface with string-literal
    // type unions. Each literal is a `literal_type` node wrapping a
    // `string` — must NOT emit TypeRef for the string content.
    let src = r#"
interface Props {
    variant?: 'default' | 'success' | 'warning' | 'danger' | 'outline';
    size?: 'sm' | 'md';
}
const { variant = 'default', size = 'sm' } = Astro.props;
"#;
    let r = extract::extract(src, false);
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    for literal in &["default", "success", "warning", "danger", "outline", "sm", "md"] {
        assert!(
            !type_refs.contains(literal),
            "string-literal `'{literal}'` must not emit a TypeRef; type_refs: {type_refs:?}"
        );
    }
}

#[test]
fn coverage_arrow_function_const_provider_emits_chain() {
    // The real-world pattern from ts-rallly's poll-context.tsx:
    //   export const PollContextProvider = (props) => {
    //     return <PollContext.Provider value={v}>{children}</PollContext.Provider>;
    //   };
    let r = extract::extract(
        "import React from 'react';\n\
         const PollContext = React.createContext(null);\n\
         export const PollContextProvider = (props) => {\n\
             return <PollContext.Provider value={1}>{props.children}</PollContext.Provider>;\n\
         };",
        true,
    );
    let provider_ref = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == "Provider")
        .unwrap_or_else(|| panic!(
            "arrow-const Provider ref must be emitted; refs: {:?}",
            r.refs.iter().map(|r| (r.kind, r.target_name.clone())).collect::<Vec<_>>()
        ));
    let chain = provider_ref.chain.as_ref().expect("chain must be set");
    assert_eq!(chain.segments[0].name, "PollContext");
    assert_eq!(chain.segments[1].name, "Provider");
}

#[test]
fn coverage_create_context_destructured_then_provider() {
    // Named-import createContext + <Foo.Provider> — common React pattern.
    let r = extract::extract(
        "import { createContext } from 'react';\n\
         const MyCtx = createContext(null);\n\
         function W() { return <MyCtx.Provider value={1}>x</MyCtx.Provider>; }",
        true,
    );
    // Note: we ASSERT the ref is emitted. Resolution (to Context.Provider)
    // is the resolver's job and covered by resolve tests.
    let provider_ref = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == "Provider")
        .expect("Provider Calls ref must be emitted");
    let chain = provider_ref.chain.as_ref().expect("chain must be set");
    assert_eq!(chain.segments[0].name, "MyCtx");
    assert_eq!(chain.segments[1].name, "Provider");
}

#[test]
fn coverage_jsx_context_provider_emits_chain() {
    // React Context pattern — essential for any non-trivial React codebase.
    // `<PollContext.Provider value={x}>` must emit a Calls ref for
    // `Provider` carrying the chain `[PollContext, Provider]` so the chain
    // walker can resolve it through `PollContext`'s inferred
    // `React.Context<T>` type to the Provider member on that interface.
    let r = extract::extract(
        "function Wrap() { return <PollContext.Provider value={1}>x</PollContext.Provider>; }",
        true,
    );
    let provider_ref = r
        .refs
        .iter()
        .find(|r| r.kind == EdgeKind::Calls && r.target_name == "Provider")
        .unwrap_or_else(|| panic!(
            "expected Calls ref with target_name=Provider for Context.Provider JSX; got: {:?}",
            r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
        ));
    let chain = provider_ref.chain.as_ref().expect("Provider ref must carry a chain");
    let seg_names: Vec<&str> = chain.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        seg_names,
        vec!["PollContext", "Provider"],
        "chain must be [PollContext, Provider] for <PollContext.Provider>"
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

// ---------------------------------------------------------------------------
// Gap-closure tests — new coverage from extract.rs changes
// ---------------------------------------------------------------------------

#[test]
fn coverage_lexical_declaration_inside_function_body() {
    // `const` inside a function body should produce a Variable symbol.
    let r = extract::extract(
        "function run() { const db: Database = connect(); }",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "db"),
        "lexical_declaration inside function body should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_lexical_declaration_inside_if_block() {
    // `const` inside an if block (inside a function) should still produce a Variable symbol.
    let r = extract::extract(
        "function run(flag: boolean) { if (flag) { const result: QueryResult = fetch(); } }",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "result"),
        "lexical_declaration inside if block should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_call_expression_in_arrow_function_argument() {
    // Calls inside arrow function arguments passed to other calls should be captured.
    let r = extract::extract(
        "function run(items: Item[]) { items.forEach(item => processItem(item)); }",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "processItem"),
        "call_expression inside arrow function argument should produce Calls ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_identifier_in_nested_context() {
    // type_identifier in a complex type expression that may not go through a
    // dedicated type_annotation handler should still produce a TypeRef.
    let r = extract::extract(
        "const x: Array<UserProfile> = [];",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "UserProfile"),
        "type_identifier in generic type argument should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_property_signature_in_type_alias_object_type() {
    // property_signature inside a type alias object literal should produce a Property symbol.
    let r = extract::extract(
        "type Config = { host: string; port: number; };",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "host"),
        "property_signature in type alias object_type should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_signature_in_type_alias_object_type() {
    // method_signature inside a type alias object literal should produce a Method symbol.
    let r = extract::extract(
        "type Service = { find(id: number): User; };",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "find"),
        "method_signature in type alias object_type should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_as_expression_deeply_nested() {
    // as_expression inside a return statement inside a method body.
    let r = extract::extract(
        "class Svc { handle(x: unknown) { return (x as AdminUser).doStuff(); } }",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "AdminUser"),
        "as_expression deeply nested should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_binary_expression_instanceof_at_module_scope() {
    // instanceof at module scope (not inside a function) should produce TypeRef.
    let r = extract::extract(
        "const isAdmin = user instanceof AdminUser;",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "AdminUser"),
        "instanceof at module scope should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_annotation_in_arrow_function_param() {
    // Type annotation on an arrow function parameter should produce TypeRef.
    let r = extract::extract(
        "const handler = (req: Request) => req.body;",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "Request"),
        "type_annotation in arrow function param should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_satisfies_expression_generic_type() {
    // satisfies with a generic type should extract both the base type and type args.
    let r = extract::extract(
        "const m = new Map() satisfies Map<string, UserEntry>;",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "UserEntry"),
        "satisfies_expression with generic type arg should produce TypeRef for type arg; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_satisfies_expression_union_type() {
    // satisfies with a union type should extract all arms.
    let r = extract::extract(
        "const val = data satisfies AdminUser | GuestUser;",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "AdminUser"),
        "satisfies_expression with union type should produce TypeRef for first arm; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "GuestUser"),
        "satisfies_expression with union type should produce TypeRef for second arm; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_enum_body_removed_from_symbol_node_kinds() {
    // enum_body is a container, not a symbol. Verify it is NOT in symbol_node_kinds.
    use crate::languages::LanguagePlugin;
    use super::TypeScriptPlugin;
    let plugin = TypeScriptPlugin;
    assert!(
        !plugin.symbol_node_kinds().contains(&"enum_body"),
        "enum_body should not be in symbol_node_kinds (it is a container, not a symbol)"
    );
}

#[test]
fn coverage_binary_expression_removed_from_ref_node_kinds() {
    // binary_expression is too broad (mostly arithmetic). instanceof is handled inline.
    // Verify binary_expression is NOT in ref_node_kinds.
    use crate::languages::LanguagePlugin;
    use super::TypeScriptPlugin;
    let plugin = TypeScriptPlugin;
    assert!(
        !plugin.ref_node_kinds().contains(&"binary_expression"),
        "binary_expression should not be in ref_node_kinds (too broad; instanceof handled inline)"
    );
}

#[test]
fn coverage_instanceof_still_works_after_binary_expression_removal() {
    // Confirm instanceof still emits TypeRef even though binary_expression is no longer
    // listed in ref_node_kinds — the extract_node arm handles it directly.
    let r = extract::extract(
        "function check(x: unknown) { if (x instanceof ServiceError) {} }",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "ServiceError"),
        "instanceof should still produce TypeRef via inline extract_node handling; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Gap-closure tests — object_type recursion for sym gaps
// ---------------------------------------------------------------------------

#[test]
fn coverage_property_signature_in_union_type_alias() {
    // property_signature inside a union member of a type alias should produce symbols.
    let r = extract::extract(
        "type T = { host: string } | { port: number };",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "host"),
        "property_signature in union object_type should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "port"),
        "property_signature in second union object_type should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_property_signature_in_intersection_type_alias() {
    // property_signature inside an intersection member of a type alias should produce symbols.
    let r = extract::extract(
        "type T = BaseType & { extra: string };",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "extra"),
        "property_signature in intersection object_type should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_signature_in_union_type_alias() {
    // method_signature inside a union member should produce Method symbol.
    let r = extract::extract(
        "type Service = { find(id: number): User } | { search(q: string): User[] };",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "find"),
        "method_signature in union object_type should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_call_signature_in_type_alias() {
    // call_signature inside a type alias object_type should produce a Method symbol.
    let r = extract::extract(
        "type Callable = { (x: number): string; };",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "call"),
        "call_signature in type alias object_type should produce Method symbol named 'call'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_index_signature_in_type_alias() {
    // index_signature inside a type alias object_type should produce a Property symbol.
    let r = extract::extract(
        "type Lookup = { [key: string]: User };",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name.contains("key")),
        "index_signature in type alias object_type should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Gap-closure tests — ref coverage for type_annotation / as_expression /
// satisfies_expression in deeply nested expression contexts
// ---------------------------------------------------------------------------

#[test]
fn coverage_type_annotation_in_ternary_arrow_param() {
    // type_annotation in an arrow function inside a ternary — deeply nested.
    let r = extract::extract(
        "const h = flag ? (req: Request) => req.url : (req: Request) => null;",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "Request"),
        "type_annotation in ternary arrow param should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_as_expression_in_ternary() {
    // as_expression inside a ternary should produce TypeRef.
    let r = extract::extract(
        "const x = flag ? (val as AdminUser) : null;",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "AdminUser"),
        "as_expression in ternary should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_as_expression_in_array_literal() {
    // as_expression inside an array literal.
    let r = extract::extract(
        "const items = [x as Widget, y as Widget];",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "Widget"),
        "as_expression in array literal should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_satisfies_expression_at_module_scope() {
    // satisfies_expression at module scope should produce TypeRef.
    let r = extract::extract(
        "const config = { debug: false } satisfies AppConfig;",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "AppConfig"),
        "satisfies_expression at module scope should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_satisfies_expression_in_function_body() {
    // satisfies_expression inside a function body.
    let r = extract::extract(
        "function setup() { return { key: 'val' } satisfies ServiceConfig; }",
        false,
    );
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "ServiceConfig"),
        "satisfies_expression in function body should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
#[ignore]
fn debug_inline_object_type_in_function_param() {
    let r = extract::extract("function foo(opts: { x: number; y: string }) {}", false);
    eprintln!("Symbols: {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
    // Should have `x` and `y` as Property symbols
    assert!(r.symbols.iter().any(|s| s.name == "x"), "expected x; got {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn debug_inline_object_type_in_var_annotation() {
    let r = extract::extract("const config: { host: string; port: number } = {} as any;", false);
    eprintln!("Symbols: {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
    assert!(r.symbols.iter().any(|s| s.name == "host"), "expected host; got {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn debug_inline_object_type_in_return_type() {
    let r = extract::extract("function bar(): { id: number } { return { id: 1 }; }", false);
    eprintln!("Symbols return type: {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn debug_inline_object_type_in_method_param() {
    let r = extract::extract("interface IRepo { find(opts: { id: number }): User; }", false);
    eprintln!("Symbols method sig: {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn debug_inline_object_type_in_method_def() {
    let r = extract::extract("class Svc { handle(opts: { x: number }): void {} }", false);
    eprintln!("Symbols method def: {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
}

// ---------------------------------------------------------------------------
// Gap-closure tests — property_signature with nested object-type annotations
// (Prisma-style interfaces / nested delegate patterns)
// ---------------------------------------------------------------------------

#[test]
fn coverage_method_signature_in_property_signature_object_type() {
    // Prisma-style: interface property whose type is an object type containing
    // method_signatures. Previously only the property was extracted; now the
    // nested method signatures must also produce Method symbols.
    let r = extract::extract(
        r#"
interface PrismaClient {
    user: {
        findUnique(args: FindUniqueArgs): Promise<User | null>;
        findMany(args?: FindManyArgs): User[];
        create(args: CreateArgs): User;
    };
}
"#,
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "findUnique"),
        "method_signature nested in property_signature object_type should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "findMany"),
        "second method_signature nested in property_signature object_type should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_property_signature_in_class_field_object_type() {
    // Class field whose type is an object type containing property signatures.
    let r = extract::extract(
        "class Svc { private ops: { findOne(): User; deleteById(id: number): void; }; }",
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "findOne"),
        "method_signature nested in class field object_type should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_property_signature_in_property_signature_object_type() {
    // Nested property signatures: interface property whose type contains property signatures.
    let r = extract::extract(
        r#"
interface Config {
    server: {
        host: string;
        port: number;
        ssl: boolean;
    };
}
"#,
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "host"),
        "property_signature nested in property_signature object_type should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "port"),
        "second property_signature nested in property_signature object_type should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_multiline_method_signature_in_interface() {
    // Multiline method signature in interface — the symbol start_line must match
    // the method_signature CST node's start line for correlation to work.
    let r = extract::extract(
        r#"export interface Calendar {
  getCredentialId?(): number;
  createEvent(
    event: CalendarEvent,
    credentialId: number
  ): Promise<NewCalendarEventType>;
  deleteEvent(uid: string): Promise<void>;
}
"#,
        false,
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "createEvent"),
        "multiline method_signature should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "getCredentialId"),
        "optional method_signature should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
#[ignore]
fn debug_crm_interface() {
    let r = extract::extract(
        r#"export interface SalesforceCRM extends CRM {
  findUserEmailFromLookupField(
    attendeeEmail: string,
    fieldName: string,
    salesforceObject: SalesforceRecordEnum
  ): Promise<{ email: string; recordType: RoutingReasons } | undefined>;

  incompleteBookingWriteToRecord(
    email: string,
    writeToRecordObject: SomeType
  ): Promise<void>;

  getAllPossibleAccountWebsiteFromEmailDomain(emailDomain: string): string;
}
"#,
        false,
    );
    eprintln!("Symbols:");
    for s in &r.symbols {
        eprintln!("  {:?} {:?} line={}", s.kind, s.name, s.start_line);
    }
}

#[test]
#[ignore]
fn debug_real_file_method_sigs() {
    // Read and parse the actual react-calcom Calendar.d.ts file
    let path = "F:/Work/Projects/TestProjects/react-calcom/packages/types/Calendar.d.ts";
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Cannot read file: {}", e);
            return;
        }
    };
    let r = extract::extract(&src, false);
    
    // Count method_signature CST nodes
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(&src, None).unwrap();
    
    let mut method_sig_lines: Vec<u32> = Vec::new();
    {
        let mut stack: Vec<tree_sitter::Node> = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == "method_signature" {
                method_sig_lines.push(node.start_position().row as u32);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
    }
    
    let method_sym_lines: Vec<u32> = r.symbols.iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .map(|s| s.start_line)
        .collect();
    
    eprintln!("CST method_signature lines: {:?}", &method_sig_lines[..method_sig_lines.len().min(20)]);
    eprintln!("Extracted Method sym lines: {:?}", &method_sym_lines[..method_sym_lines.len().min(20)]);
    eprintln!("CST total: {}, Extracted: {}", method_sig_lines.len(), method_sym_lines.len());
    
    // Find missing lines
    let extracted_set: std::collections::HashSet<u32> = method_sym_lines.iter().copied().collect();
    let missing: Vec<u32> = method_sig_lines.iter().filter(|l| !extracted_set.contains(l)).copied().collect();
    eprintln!("Missing method_signature lines: {:?}", &missing[..missing.len().min(20)]);
    
    // Show what's at those lines
    let lines: Vec<&str> = src.lines().collect();
    for &l in missing.iter().take(10) {
        if (l as usize) < lines.len() {
            eprintln!("  Line {}: {:?}", l, lines[l as usize].trim());
        }
    }
}

#[test]
#[ignore]
fn debug_scan_project_method_sigs() {
    use std::collections::HashSet;
    
    let project_root = "F:/Work/Projects/TestProjects/react-calcom";
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    
    let mut total_cst = 0u64;
    let mut total_extracted = 0u64;
    let mut missing_examples: Vec<(String, u32, String)> = Vec::new();

    let walk_result = crate::walker::walk(std::path::Path::new(project_root));
    let files: Vec<crate::walker::WalkedFile> = match walk_result {
        Ok(f) => f,
        Err(e) => { eprintln!("Walk error: {}", e); return; }
    };

    let ts_files: Vec<_> = files.iter().filter(|f| f.language == "typescript").collect();
    eprintln!("TypeScript files: {}", ts_files.len());
    
    for walked in ts_files.iter().take(200) {
        let src = match std::fs::read_to_string(&walked.absolute_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&language).is_err() { continue; }
        let tree = match parser.parse(&src, None) {
            Some(t) => t,
            None => continue,
        };
        
        let mut cst_lines: Vec<u32> = Vec::new();
        // Stack-based traversal to count method_signature nodes.
        let mut stack: Vec<tree_sitter::Node> = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == "method_signature" {
                cst_lines.push(node.start_position().row as u32);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        
        if cst_lines.is_empty() { continue; }
        
        let r = super::extract::extract(&src, walked.relative_path.ends_with(".tsx"));
        let extracted_set: HashSet<u32> = r.symbols.iter()
            .filter(|s| s.kind == crate::types::SymbolKind::Method)
            .map(|s| s.start_line)
            .collect();
        
        total_cst += cst_lines.len() as u64;
        
        let file_lines: Vec<&str> = src.lines().collect();
        for &l in &cst_lines {
            if extracted_set.contains(&l) {
                total_extracted += 1;
            } else if missing_examples.len() < 10 {
                let line_text = if (l as usize) < file_lines.len() {
                    file_lines[l as usize].trim().to_string()
                } else {
                    "?".to_string()
                };
                missing_examples.push((walked.relative_path.clone(), l, line_text));
            }
        }
    }
    
    eprintln!("Total CST method_signature: {}", total_cst);
    eprintln!("Total extracted: {}", total_extracted);
    eprintln!("Missing examples:");
    for (path, line, text) in &missing_examples {
        eprintln!("  {}:{}: {:?}", path, line, text);
    }
}

#[test]
#[ignore]
fn debug_tsx_method_sigs() {
    let r = extract::extract(
        r#"
interface Handler {
  handle(req: Request, res: Response): void;
  validate(data: unknown): boolean;
}
type Props = {
  onClick(): void;
  onHover(e: MouseEvent): void;
};
"#,
        true,  // TSX grammar
    );
    eprintln!("TSX symbols:");
    for s in &r.symbols {
        eprintln!("  {:?} {:?} line={}", s.kind, s.name, s.start_line);
    }
}

#[test]
#[ignore]
fn debug_find_files_with_missing_method_sigs() {
    let project_root = "F:/Work/Projects/TestProjects/react-calcom";
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    
    let walk_result = crate::walker::walk(std::path::Path::new(project_root));
    let files: Vec<crate::walker::WalkedFile> = match walk_result {
        Ok(f) => f,
        Err(e) => { eprintln!("Walk error: {}", e); return; }
    };
    let ts_files: Vec<_> = files.iter().filter(|f| f.language == "typescript").collect();
    eprintln!("Total TS files: {}", ts_files.len());
    
    let mut problem_files: Vec<(String, usize, usize)> = Vec::new(); // (path, cst, extracted)
    
    for walked in &ts_files {
        let src = match std::fs::read_to_string(&walked.absolute_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&language).is_err() { continue; }
        let tree = match parser.parse(&src, None) {
            Some(t) => t,
            None => continue,
        };
        
        // Count method_signature nodes using stack
        let mut cst_lines: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
        let mut stack: Vec<tree_sitter::Node> = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == "method_signature" {
                cst_lines.insert(node.start_position().row as u32);
            }
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                stack.push(child);
            }
        }
        
        if cst_lines.is_empty() { continue; }
        
        let is_tsx = walked.relative_path.ends_with(".tsx");
        let r = super::extract::extract(&src, is_tsx);
        let extracted_lines: std::collections::BTreeSet<u32> = r.symbols.iter()
            .filter(|s| s.kind == crate::types::SymbolKind::Method)
            .map(|s| s.start_line)
            .collect();
        
        let missing_count = cst_lines.difference(&extracted_lines).count();
        if missing_count > 0 {
            problem_files.push((walked.relative_path.clone(), cst_lines.len(), missing_count));
        }
    }
    
    problem_files.sort_by_key(|f| std::cmp::Reverse(f.2));
    eprintln!("Files with missing method_signatures (top 20):");
    for (path, total, missing) in problem_files.iter().take(20) {
        eprintln!("  {} total={}, missing={}", path, total, missing);
    }
}

#[test]
#[ignore]
fn debug_tsx_file_method_sigs() {
    let path = "F:/Work/Projects/TestProjects/react-calcom/apps/web/modules/auth/login-view.tsx";
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("Cannot read: {}", e); return; }
    };
    
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(&src, None).unwrap();
    
    let lines: Vec<&str> = src.lines().collect();
    let mut stack: Vec<tree_sitter::Node> = vec![tree.root_node()];
    let mut method_sig_nodes: Vec<(u32, String)> = Vec::new();
    while let Some(node) = stack.pop() {
        if node.kind() == "method_signature" {
            let line = node.start_position().row as u32;
            let text = if (line as usize) < lines.len() {
                lines[line as usize].trim().to_string()
            } else {
                "?".to_string()
            };
            method_sig_nodes.push((line, text));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    
    eprintln!("method_signature nodes in login-view.tsx:");
    for (line, text) in &method_sig_nodes[..method_sig_nodes.len().min(20)] {
        eprintln!("  line {}: {:?}", line, text);
    }
    
    let r = super::extract::extract(&src, true); // TSX
    let method_syms: Vec<_> = r.symbols.iter().filter(|s| s.kind == SymbolKind::Method).collect();
    eprintln!("Extracted Method symbols: {}", method_syms.len());
    for s in &method_syms {
        eprintln!("  {:?} line={}", s.name, s.start_line);
    }
}

#[test]
#[ignore]
fn debug_ts_grammar_on_tsx_file() {
    // Check what LANGUAGE_TYPESCRIPT sees in a TSX file
    let path = "F:/Work/Projects/TestProjects/react-calcom/apps/web/modules/auth/login-view.tsx";
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => { eprintln!("Cannot read: {}", e); return; }
    };
    
    // Use TYPESCRIPT grammar (as coverage check does)
    let language_ts: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser_ts = tree_sitter::Parser::new();
    parser_ts.set_language(&language_ts).unwrap();
    let tree_ts = parser_ts.parse(&src, None).unwrap();
    
    let lines: Vec<&str> = src.lines().collect();
    let mut ts_method_sigs: Vec<(u32, String)> = Vec::new();
    let mut stack: Vec<tree_sitter::Node> = vec![tree_ts.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == "method_signature" {
            let line = node.start_position().row as u32;
            let text = if (line as usize) < lines.len() {
                lines[line as usize].trim().to_string()
            } else {
                "?".to_string()
            };
            ts_method_sigs.push((line, text));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    
    eprintln!("TYPESCRIPT grammar method_signature count: {}", ts_method_sigs.len());
    for (line, text) in ts_method_sigs.iter().take(10) {
        eprintln!("  line {}: {:?}", line, text);
    }
    
    // Use TSX grammar (as extractor does for .tsx files)
    let language_tsx: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TSX.into();
    let mut parser_tsx = tree_sitter::Parser::new();
    parser_tsx.set_language(&language_tsx).unwrap();
    let tree_tsx = parser_tsx.parse(&src, None).unwrap();
    
    let mut tsx_method_sigs: Vec<(u32, String)> = Vec::new();
    let mut stack2: Vec<tree_sitter::Node> = vec![tree_tsx.root_node()];
    while let Some(node) = stack2.pop() {
        if node.kind() == "method_signature" {
            let line = node.start_position().row as u32;
            let text = if (line as usize) < lines.len() {
                lines[line as usize].trim().to_string()
            } else {
                "?".to_string()
            };
            tsx_method_sigs.push((line, text));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack2.push(child);
        }
    }
    
    eprintln!("TSX grammar method_signature count: {}", tsx_method_sigs.len());
    for (line, text) in tsx_method_sigs.iter().take(10) {
        eprintln!("  line {}: {:?}", line, text);
    }
}

#[test]
#[ignore]
fn debug_measure_coverage_calcom() {
    let path = std::path::Path::new("F:/Work/Projects/TestProjects/react-calcom");
    if !path.exists() {
        eprintln!("Project not found, skipping");
        return;
    }
    let results = crate::query::coverage::analyze_coverage(path);
    for cov in &results {
        if cov.language == "typescript" {
            eprintln!("=== TypeScript ===");
            eprintln!("  files: {}", cov.file_count);
            eprintln!("  sym: {:.1}% ({}/{})", cov.symbol_coverage.percent, cov.symbol_coverage.matched_nodes, cov.symbol_coverage.expected_nodes);
            eprintln!("  ref: {:.1}% ({}/{})", cov.ref_coverage.percent, cov.ref_coverage.matched_nodes, cov.ref_coverage.expected_nodes);
            eprintln!("  --- symbol kinds ---");
            let mut sym_kinds = cov.symbol_kinds.clone();
            sym_kinds.sort_by(|a, b| a.percent.partial_cmp(&b.percent).unwrap());
            for k in sym_kinds.iter().take(10) {
                eprintln!("    {}: {:.1}% ({}/{}) miss={}", k.kind, k.percent, k.matched, k.occurrences, k.occurrences - k.matched);
            }
            eprintln!("  --- ref kinds ---");
            let mut ref_kinds = cov.ref_kinds.clone();
            ref_kinds.sort_by(|a, b| a.percent.partial_cmp(&b.percent).unwrap());
            for k in ref_kinds.iter().take(10) {
                eprintln!("    {}: {:.1}% ({}/{}) miss={}", k.kind, k.percent, k.matched, k.occurrences, k.occurrences - k.matched);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// New coverage — node types from rules not yet exercised above
// ---------------------------------------------------------------------------

#[test]
fn coverage_function_expression_symbol() {
    // `const f = function() {}` -- function_expression initializer.
    // push_variable_decl promotes to Function kind when the initializer is a
    // function_expression or arrow_function.
    let r = extract::extract("const format = function(x: string): string { return x; };", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "format"),
        "function_expression in variable_declarator should produce a Function symbol named 'format'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_arrow_function_symbol() {
    // `const fn = (x: T) => x` -- arrow_function in variable_declarator.
    // push_variable_decl promotes to Function kind when the initializer is an arrow_function.
    let r = extract::extract("const transform = (x: number): number => x * 2;", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "transform"),
        "arrow_function in variable_declarator should produce a Function symbol named 'transform'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_constructor_symbol() {
    // method_definition named "constructor" should produce Constructor kind.
    let r = extract::extract("class Service { constructor(private db: Database) {} }", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Constructor && s.name == "constructor"),
        "method_definition named 'constructor' should produce Constructor symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_getter_signature() {
    // getter_signature in an interface body → extracted via push_ts_field.
    // push_ts_field uses the `name` field of the node; tree-sitter parses
    // `get size()` as a getter_signature whose name field is "size".
    // The extractor routes getter_signature through push_ts_field which currently
    // yields Method kind (the field lookup path uses SymbolKind::Method as fallback).
    let r = extract::extract("interface IStore { get size(): number; }", false);
    assert!(
        r.symbols.iter().any(|s| (s.kind == SymbolKind::Property || s.kind == SymbolKind::Method)
            && s.name == "size"),
        "getter_signature should produce Property or Method symbol named 'size'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_setter_signature() {
    // setter_signature in an interface body → extracted via push_ts_field.
    // Same as getter_signature: the extractor yields Method kind via push_ts_field.
    let r = extract::extract("interface IStore { set value(v: string); }", false);
    assert!(
        r.symbols.iter().any(|s| (s.kind == SymbolKind::Property || s.kind == SymbolKind::Method)
            && s.name == "value"),
        "setter_signature should produce Property or Method symbol named 'value'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_ambient_declaration() {
    // `declare class Foo {}` — ambient_declaration wrapping a class_declaration.
    // The extractor recurses through ambient_declaration, so the inner class produces
    // a Class symbol. (Note: `declare function foo()` uses function_signature which
    // has no body and is not yet handled as a standalone symbol.)
    let r = extract::extract("declare class Serializer {}", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "Serializer"),
        "ambient_declaration wrapping class_declaration should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_function_signature() {
    // `function name(params): ReturnType;` -- ambient / overload function signature.
    // function_signature has no body but is otherwise structurally identical to
    // function_declaration; the dedicated arm in extract_node handles it via push_function.
    let r = extract::extract("function parse(input: string): AST;\nfunction parse(input: Buffer): AST;", false);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "parse"),
        "function_signature should produce at least one Function symbol named 'parse'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_import_default() {
    // `import React from 'react'` — default (identifier) import → TypeRef with module.
    let r = extract::extract(r#"import React from 'react';"#, false);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "React"
            && r.module.as_deref() == Some("react")),
        "default import should produce TypeRef with module='react'; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name, &r.module)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_namespace_import() {
    // `import * as ns from 'module'` -- namespace_import.
    // push_import now handles namespace_import inside import_clause and emits a TypeRef
    // for the local alias with the module path set.
    let r = extract::extract(r#"import * as path from 'path';"#, false);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "path"
            && r.module.as_deref() == Some("path")),
        "namespace import should produce TypeRef with target_name='path' and module='path'; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name, &r.module)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_export_reexport_with_source() {
    // `export { Foo } from './foo'` — named re-export should emit an Imports ref with module.
    let r = extract::extract(r#"export { UserService } from './user';"#, false);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Imports && r.target_name == "UserService"
            && r.module.as_deref() == Some("./user")),
        "re-export with source should produce Imports ref with module='./user'; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name, &r.module)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_type_assertion() {
    // `<AdminUser>user` — old-style angle-bracket type assertion → TypeRef for the cast type.
    let r = extract::extract("const admin = <AdminUser>user;", false);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "AdminUser"),
        "type_assertion (<Type>expr) should produce TypeRef; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_interface_extends_type_clause() {
    // `interface B extends A` -- tree-sitter uses `extends_type_clause` for interface
    // inheritance. extract_heritage now handles it and emits an Inherits edge.
    let r = extract::extract("interface Serializable extends Printable {}", false);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Inherits && r.target_name == "Printable"),
        "interface extends_type_clause should produce Inherits ref for 'Printable'; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_call_member_expression() {
    // `obj.method()` — call_expression whose function is a member_expression.
    // Calls ref target_name should contain the method name "warn".
    let r = extract::extract("function run() { logger.warn('oops'); }", false);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Calls && r.target_name.contains("warn")),
        "method call on member_expression should produce Calls ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_import_require_clause() {
    // `import x = require("mod")` -- CommonJS-style TypeScript import_require_clause.
    // push_import now handles the import_require_clause child of import_statement and
    // emits an Imports ref with the local alias as target_name and the module path set.
    let r = extract::extract(r#"import path = require("path");"#, false);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Imports && r.target_name == "path"
            && r.module.as_deref() == Some("path")),
        "import_require_clause should produce Imports ref with target_name='path' and module='path'; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name, &r.module)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_require_call_dynamic() {
    // `const mod = require('module')` — top-level CommonJS require.
    // TS extractor handles require via annotate_call_modules + Calls arm.
    let r = extract::extract(r#"const logger = require('winston');"#, false);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Calls || r.kind == EdgeKind::Imports),
        "require() call should produce at least one Calls or Imports ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn dts_class_method_emits_return_type_ref() {
    // Dayjs-shaped .d.ts: ambient class inside a namespace, methods have
    // return types but no bodies. This is the exact pattern dayjs,
    // moment, chai, etc. use — and the reason why `dayjs_synthetics.rs`
    // exists today. The extractor must emit a TypeRef from every method
    // to its return type so `engine::TypeInfo::return_type` is populated
    // and the chain walker can follow `dayjs().clone().format()`.
    let src = r#"
declare namespace dayjs {
  class Dayjs {
    constructor(config?: string)
    clone(): Dayjs
    isValid(): boolean
    year(): number
    year(value: number): Dayjs
    format(template?: string): string
  }
}
"#;
    let r = extract::extract(src, false);

    // Symbol sanity: the method symbols must exist.
    let clone_idx = r
        .symbols
        .iter()
        .position(|s| s.qualified_name.ends_with("Dayjs.clone") && s.kind == SymbolKind::Method)
        .expect("Dayjs.clone method symbol missing");
    let year_overloads: Vec<_> = r
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.qualified_name.ends_with("Dayjs.year") && s.kind == SymbolKind::Method)
        .collect();
    assert!(
        !year_overloads.is_empty(),
        "Dayjs.year overloads missing; symbols: {:?}",
        r.symbols.iter().map(|s| &s.qualified_name).collect::<Vec<_>>()
    );

    // Return-type TypeRef: clone(): Dayjs must produce TypeRef target_name=Dayjs.
    let clone_return_refs: Vec<_> = r
        .refs
        .iter()
        .filter(|x| x.source_symbol_index == clone_idx && x.kind == EdgeKind::TypeRef)
        .collect();
    assert!(
        clone_return_refs.iter().any(|x| x.target_name == "Dayjs"),
        "clone(): Dayjs should emit TypeRef to Dayjs; got refs: {:?}",
        clone_return_refs
            .iter()
            .map(|x| (&x.target_name, x.kind))
            .collect::<Vec<_>>()
    );
}
