use super::extract;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use std::collections::BTreeSet;

fn sym(source: &str) -> Vec<ExtractedSymbol> { extract::extract(source, false).symbols }
fn refs(source: &str) -> Vec<ExtractedRef>    { extract::extract(source, false).refs }

#[test]
fn extracts_class() {
    let src = "export class UserService {}";
    let symbols = sym(src);
    let s = symbols.iter().find(|s| s.name == "UserService").unwrap();
    assert_eq!(s.kind, SymbolKind::Class);
}

#[test]
fn extracts_interface() {
    let src = "interface IRepository { save(): void; }";
    let symbols = sym(src);
    let i = symbols.iter().find(|s| s.name == "IRepository").unwrap();
    assert_eq!(i.kind, SymbolKind::Interface);
}

#[test]
fn extracts_function() {
    let src = "function fetchUsers(): Promise<User[]> { return []; }";
    let symbols = sym(src);
    let f = symbols.iter().find(|s| s.name == "fetchUsers").unwrap();
    assert_eq!(f.kind, SymbolKind::Function);
}

#[test]
fn extracts_method() {
    let src = "class Svc { async getById(id: number) {} }";
    let symbols = sym(src);
    let m = symbols.iter().find(|s| s.name == "getById").unwrap();
    assert_eq!(m.kind, SymbolKind::Method);
}

#[test]
fn extracts_type_alias() {
    let src = "type UserId = string;";
    let symbols = sym(src);
    let t = symbols.iter().find(|s| s.name == "UserId").unwrap();
    assert_eq!(t.kind, SymbolKind::TypeAlias);
}

#[test]
fn extracts_extends_as_inherits() {
    let src = "class Foo extends Bar {}";
    let r = refs(src);
    assert!(r.iter().any(|r| r.target_name == "Bar" && r.kind == EdgeKind::Inherits),
        "refs: {r:?}");
}

#[test]
fn extracts_import() {
    let src = r#"import { CatalogService } from "./catalog";"#;
    let r = refs(src);
    assert!(r.iter().any(|r| r.target_name == "CatalogService"), "refs: {r:?}");
    let imp = r.iter().find(|r| r.target_name == "CatalogService").unwrap();
    assert_eq!(imp.module, Some("./catalog".to_string()));
}

#[test]
fn extracts_call() {
    let src = "function run() { fetchData(); }";
    let r = refs(src);
    let calls: Vec<_> = r.iter().filter(|r| r.kind == EdgeKind::Calls).collect();
    assert!(calls.iter().any(|r| r.target_name == "fetchData"), "calls: {calls:?}");
}

#[test]
fn qualified_name_includes_class() {
    let src = "class Catalog { list(): void {} }";
    let symbols = sym(src);
    let m = symbols.iter().find(|s| s.name == "list").unwrap();
    assert_eq!(m.qualified_name, "Catalog.list");
}

#[test]
fn does_not_panic_on_malformed_source() {
    let src = "class { !!! broken @@@ }";
    let _ = extract::extract(src, false);
}

// ---------------------------------------------------------------------------
// Type assertions
// ---------------------------------------------------------------------------

#[test]
fn as_expression_emits_type_ref() {
    // `const admin = user as Admin` — asserted type should be a TypeRef.
    let src = "const admin = user as Admin;";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
        "refs: {r:?}"
    );
}

#[test]
fn as_expression_generic_emits_base_type_ref() {
    // `const repo = raw as Repository<User>` — base type should be emitted.
    let src = "const repo = raw as Repository<User>;";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Repository" && r.kind == EdgeKind::TypeRef),
        "refs: {r:?}"
    );
}

#[test]
fn type_assertion_emits_type_ref() {
    // TSX angle-bracket form is not valid in .tsx files but is valid in .ts.
    // `const admin = <Admin>user` — asserted type should be a TypeRef.
    let src = "const admin = <Admin>user;";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
        "refs: {r:?}"
    );
}

// ---------------------------------------------------------------------------
// Catch clause variables
// ---------------------------------------------------------------------------

#[test]
fn catch_clause_typed_emits_variable_and_type_ref() {
    let src = "try { doWork(); } catch (e: Error) { console.log(e); }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "e" && s.kind == SymbolKind::Variable),
        "symbols: {s:?}"
    );
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Error" && r.kind == EdgeKind::TypeRef),
        "refs: {r:?}"
    );
}

// ---------------------------------------------------------------------------
// instanceof narrowing
// ---------------------------------------------------------------------------

#[test]
fn instanceof_in_if_emits_type_ref() {
    let src = "function check(user: unknown) { if (user instanceof Admin) { user.doStuff(); } }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
        "refs: {r:?}"
    );
}

#[test]
fn instanceof_in_method_emits_type_ref() {
    let src = "class Svc { handle(x: unknown) { if (x instanceof Handler) {} } }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Handler" && r.kind == EdgeKind::TypeRef),
        "refs: {r:?}"
    );
}

#[test]
fn as_expression_in_call_chain_emits_type_ref() {
    // `(user as Admin).doAdminStuff()` — as_expression in expression context, not var decl.
    let src = "function go(user: unknown) { (user as Admin).doAdminStuff(); }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
        "refs: {r:?}"
    );
}

#[test]
fn catch_clause_untyped_emits_variable_only() {
    let src = "try { doWork(); } catch (e) { console.log(e); }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "e" && s.kind == SymbolKind::Variable),
        "symbols: {s:?}"
    );
    // No TypeRef expected — untyped catch variable.
    let r = refs(src);
    let type_refs_to_e: Vec<_> =
        r.iter().filter(|r| r.kind == EdgeKind::TypeRef && r.target_name == "e").collect();
    assert!(type_refs_to_e.is_empty(), "unexpected TypeRef for untyped catch: {type_refs_to_e:?}");
}

// ---------------------------------------------------------------------------
// Namespace / internal_module
// ---------------------------------------------------------------------------

#[test]
fn namespace_emits_namespace_symbol() {
    let src = "namespace MyNS { export const x = 1; }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "MyNS" && s.kind == SymbolKind::Namespace),
        "symbols: {s:?}"
    );
}

#[test]
fn namespace_members_are_nested() {
    let src = "namespace MyNS { export function helper() {} }";
    let s = sym(src);
    // Both symbols must be present.
    assert!(s.iter().any(|s| s.name == "MyNS" && s.kind == SymbolKind::Namespace), "no MyNS: {s:?}");
    let helper = s.iter().find(|s| s.name == "helper").unwrap();
    // The helper should be parented to the namespace (parent_index is Some).
    assert!(helper.parent_index.is_some(), "helper should have a parent_index");
    // The helper's qualified name should contain the helper name.
    assert!(helper.qualified_name.contains("helper"), "qname: {}", helper.qualified_name);
}

// ---------------------------------------------------------------------------
// Generator functions
// ---------------------------------------------------------------------------

#[test]
fn generator_function_declaration_emits_function_symbol() {
    let src = "function* gen(): Generator<number> { yield 1; }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "gen" && s.kind == SymbolKind::Function),
        "symbols: {s:?}"
    );
}

#[test]
fn generator_method_emits_method_symbol() {
    let src = "class C { async *gen(): AsyncGenerator<number> { yield 1; } }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name == "gen" && s.kind == SymbolKind::Method),
        "symbols: {s:?}"
    );
}

// ---------------------------------------------------------------------------
// Tagged template expression calls
// ---------------------------------------------------------------------------

#[test]
fn tagged_template_emits_call_ref() {
    let src = "function run() { const q = sql`SELECT * FROM users`; }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "sql" && r.kind == EdgeKind::Calls),
        "refs: {r:?}"
    );
}

#[test]
fn tagged_template_member_chain_emits_call() {
    // `gql` tagged template — tag is an identifier.
    let src = "function run() { const q = gql`query { users { id } }`; }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "gql" && r.kind == EdgeKind::Calls),
        "refs: {r:?}"
    );
}

// ---------------------------------------------------------------------------
// Advanced type forms (types.rs additions)
// ---------------------------------------------------------------------------

#[test]
fn conditional_type_emits_type_refs() {
    // `type R<T> = T extends string ? 'str' : 'other'`
    // → TypeRef to string (the extends type) from R.
    let src = "type R<T> = T extends Foo ? A : B;";
    let r = refs(src);
    // Foo, A, B are all referenced types.
    assert!(
        r.iter().any(|r| r.target_name == "Foo" && r.kind == EdgeKind::TypeRef),
        "missing Foo TypeRef: {r:?}"
    );
}

#[test]
fn keyof_type_emits_type_ref() {
    let src = "type Keys<T> = keyof T;";
    // T is a type parameter, but when used concretely:
    let src2 = "type Keys = keyof User;";
    let r = refs(src2);
    assert!(
        r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
        "missing User TypeRef: {r:?}"
    );
}

#[test]
fn typeof_type_emits_type_ref() {
    let src = "type T = typeof MyClass;";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "MyClass" && r.kind == EdgeKind::TypeRef),
        "missing MyClass TypeRef: {r:?}"
    );
}

#[test]
fn predicate_type_emits_type_ref() {
    let src = "function isAdmin(x: unknown): x is Admin { return true; }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Admin" && r.kind == EdgeKind::TypeRef),
        "missing Admin TypeRef from predicate: {r:?}"
    );
}

#[test]
fn readonly_type_emits_inner_type_ref() {
    let src = "type R = readonly User[];";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
        "missing User TypeRef from readonly_type: {r:?}"
    );
}

// ---------------------------------------------------------------------------
// Interface construct/call signatures and abstract method signatures
// ---------------------------------------------------------------------------

#[test]
fn construct_signature_emits_method_symbol() {
    let src = "interface Factory { new(name: string): Product; }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.kind == SymbolKind::Method || s.kind == SymbolKind::Constructor),
        "no construct_signature symbol: {s:?}"
    );
}

#[test]
fn index_signature_emits_property_symbol() {
    let src = "interface Lookup { [key: string]: User; }";
    let s = sym(src);
    assert!(
        s.iter().any(|s| s.name.contains("key") && s.kind == SymbolKind::Property),
        "no index_signature symbol: {s:?}"
    );
}

#[test]
fn index_signature_emits_type_ref_for_value() {
    let src = "interface Lookup { [key: string]: User; }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "User" && r.kind == EdgeKind::TypeRef),
        "missing User TypeRef from index_signature: {r:?}"
    );
}

// ---------------------------------------------------------------------------
// Top-level / field-initializer call extraction (new coverage)
// ---------------------------------------------------------------------------

#[test]
fn toplevel_call_emits_calls_ref() {
    // `setupDatabase();` at module scope — no enclosing function.
    let src = "setupDatabase();";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "setupDatabase" && r.kind == EdgeKind::Calls),
        "expected Calls ref for top-level call, got: {r:?}"
    );
}

#[test]
fn field_initializer_call_emits_calls_ref() {
    // `private logger = createLogger()` — call inside a class field initializer.
    let src = "class Svc { private logger = createLogger(); }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "createLogger" && r.kind == EdgeKind::Calls),
        "expected Calls ref from field initializer, got: {r:?}"
    );
}

#[test]
fn new_expression_at_toplevel_emits_instantiates_ref() {
    // `new EventEmitter()` at module scope.
    let src = "const emitter = new EventEmitter();";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "EventEmitter" && r.kind == EdgeKind::Instantiates),
        "expected Instantiates ref for new_expression, got: {r:?}"
    );
}

#[test]
fn new_expression_in_method_body_emits_instantiates_ref() {
    // `new Error(msg)` inside a method body — handled via extract_calls.
    let src = "class Svc { fail(msg: string) { throw new Error(msg); } }";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "Error" && r.kind == EdgeKind::Instantiates),
        "expected Instantiates ref for new Error inside method, got: {r:?}"
    );
}

#[test]
fn method_call_chain_toplevel_emits_calls_ref() {
    // `this.repo.findOne(1)` at module scope is unusual but shouldn't panic.
    // More usefully: `console.log(x)` at the top level of a script.
    let src = "console.log('hello');";
    let r = refs(src);
    assert!(
        r.iter().any(|r| r.target_name == "log" && r.kind == EdgeKind::Calls),
        "expected Calls ref for console.log at module scope, got: {r:?}"
    );
}

// ---------------------------------------------------------------------------
// Node-kind coverage diagnostic
// ---------------------------------------------------------------------------

/// Collect ALL node kinds that appear in a tree-sitter parse tree.
fn collect_all_kinds(node: tree_sitter::Node, out: &mut BTreeSet<String>) {
    out.insert(node.kind().to_string());
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_all_kinds(child, out);
    }
}

/// Node kinds handled explicitly in `extract_node`.
/// Update this set whenever extract_node gains or loses a match arm.
fn handled_kinds() -> BTreeSet<String> {
    [
        // Top-level declarations
        "class_declaration",
        "interface_declaration",
        "function_declaration",
        "export_statement",
        "method_definition",
        "public_field_definition",
        "field_definition",
        "property_signature",
        "method_signature",
        "type_alias_declaration",
        "enum_declaration",
        "lexical_declaration",
        "variable_declaration",
        "import_statement",
        "for_in_statement",
        "catch_clause",
        // Call/instantiation extraction at any level
        "call_expression",
        "new_expression",
        // New handlers (Step 2)
        "abstract_class_declaration",
        "abstract_method_signature",
        "getter_signature",
        "setter_signature",
        "construct_signature",
        "call_signature",
        "index_signature",
        "ambient_declaration",
        "internal_module",
        "module",
        "namespace_import",
        "export_clause",
        "namespace_export",
        "generator_function_declaration",
        "function_signature",
        "try_statement",
        "statement_block",
        "if_statement",
        "else_clause",
        "return_statement",
        "expression_statement",
        "throw_statement",
        "for_statement",
        "while_statement",
        "do_statement",
        "switch_statement",
        "switch_body",
        "switch_case",
        "switch_default",
        "labeled_statement",
        "with_statement",
        "break_statement",
        "continue_statement",
        "debugger_statement",
        "empty_statement",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Dump specific node kinds for particular constructs.
#[test]
fn dump_specific_node_kinds() {
    let snippets = [
        ("abstract class", "abstract class Foo { abstract method(): void; }"),
        ("namespace", "namespace NS { export const x = 1; }"),
        ("function_signature", "function overloaded(x: string): string;"),
        ("getter_signature", "interface I { get prop(): string; }"),
        ("setter_signature", "interface I { set prop(v: string); }"),
        ("index_signature", "interface I { [key: string]: unknown; }"),
        ("export_clause", "export { Foo as F };"),
        ("export_star", "export * from './all';"),
        ("satisfies", "const obj = {} satisfies Config;"),
        ("abstract_method", "abstract class A { abstract method(): void; }"),
        ("construct_sig", "interface Factory { new(name: string): Product; }"),
        ("call_sig", "interface Callable { (x: number): string; }"),
        ("keyof_type", "type Keys = keyof User;"),
        ("predicate_type", "function isAdmin(x: unknown): x is Admin { return true; }"),
        ("conditional_type", "type R = Foo extends string ? A : B;"),
    ];
    for (label, src) in &snippets {
        let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let root = tree.root_node();
        println!("\n--- {label}: {src}");
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            print_node_tree(child, 0);
        }
    }
}

fn print_node_tree(node: tree_sitter::Node, depth: usize) {
    let indent = "  ".repeat(depth);
    println!("{indent}[{}] ({}-{})", node.kind(), node.start_position().row, node.end_position().row);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if depth < 3 {
            print_node_tree(child, depth + 1);
        }
    }
}

/// Dump what we currently extract from the comprehensive sample.
#[test]
fn dump_comprehensive_extraction() {
    let src = COMPREHENSIVE_SAMPLE;
    let result = extract::extract(src, false);
    let sym_names: Vec<_> = result.symbols.iter().map(|s| format!("{:?} {}", s.kind, s.name)).collect();
    let ref_targets: Vec<_> = result.refs.iter().map(|r| format!("{:?} {}", r.kind, r.target_name)).collect();
    println!("\nSymbols ({}):", sym_names.len());
    for s in &sym_names { println!("  {s}"); }
    println!("\nRefs ({}):", ref_targets.len());
    for r in &ref_targets { println!("  {r}"); }
}

const COMPREHENSIVE_SAMPLE: &str = r#"
import { Foo } from './foo';
import type { Bar } from './bar';
export { Foo as F };
export default class Main {}
export * from './all';

@Decorator class MyClass extends Base implements IFoo {
  readonly field: string = 'hello';
  static count = 0;
  #privateField = 1;

  constructor(private db: Repo<User>) { super(); }

  get name(): string { return this.field; }
  set name(v: string) { this.field = v; }

  async *generator(): AsyncGenerator<number> { yield 1; yield* other(); }

  method(x: string | null, y?: number): x is string { return typeof x === 'string'; }

  override toString() { return `${this.name}: ${this.field}`; }
}

interface Config<T extends Record<string, unknown> = {}> {
  readonly db: T;
  timeout?: number;
  [key: string]: unknown;
  method(x: T): void;
  get prop(): string;
}

type Mapped<T> = { [K in keyof T]: T[K] };
type Conditional<T> = T extends string ? 'str' : 'other';
type Template = `prefix_${string}`;
type Tuple = [string, number, ...boolean[]];

enum Status { Active = 'active', Inactive = 'inactive' }
const enum Direction { Up, Down }

function overloaded(x: string): string;
function overloaded(x: number): number;
function overloaded(x: any): any { return x; }

declare module 'express' { interface Request { user?: User; } }
declare global { interface Window { app: App; } }

const obj = { a: 1, b: 2 };
const tagged = sql`SELECT * FROM users`;
const regex = /pattern/gi;

for (const item of items) { item.process(); }
for (const key in obj) { }
for (let i = 0; i < 10; i++) {}
while (true) { break; }
do { continue; } while (false);

switch (status) {
  case 'active': break;
  default: break;
}

try { throw new Error('fail'); } catch (e: unknown) { } finally { }

namespace NS { export const x = 1; }
"#;

/// Verify that type_identifiers inside generic_type nodes are extracted.
/// This is a fix for the 66.7% → ~100% coverage gap for type_identifier.
#[test]
fn extracts_type_identifiers_from_generic_types() {
    let src = r#"
class Repo {
  items: Repository<User>[] = [];
  cached: Map<string, Item> = new Map();
}
"#;
    let all_refs = refs(src);
    let type_refs: Vec<&str> = all_refs.iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(type_refs.contains(&"Repository"), "Should extract Repository from generic");
    assert!(type_refs.contains(&"User"), "Should extract User from generic argument");
    assert!(type_refs.contains(&"Map"), "Should extract Map from generic");
    assert!(type_refs.contains(&"Item"), "Should extract Item from generic argument");
}

/// Parse the comprehensive sample, enumerate all node kinds tree-sitter
/// produces, and report which top-level-statement kinds are NOT in
/// `handled_kinds()`.  This test never fails — it's a diagnostic tool.
#[test]
fn enumerate_unhandled_node_kinds() {
    let src = COMPREHENSIVE_SAMPLE;

    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(src, None).unwrap();

    let mut all_kinds = BTreeSet::new();
    collect_all_kinds(tree.root_node(), &mut all_kinds);

    let handled = handled_kinds();

    // Report kinds that appear at the direct-child level of `program` (statement
    // positions) and are not in `handled_kinds`.  We only care about node kinds
    // that are direct program children — inner expression kinds are covered by
    // the recursive `_` fallthrough in extract_node or handled elsewhere.
    let mut program_child_kinds = BTreeSet::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        program_child_kinds.insert(child.kind().to_string());
    }

    let unhandled: Vec<_> = program_child_kinds.difference(&handled).collect();

    if !unhandled.is_empty() {
        println!("\nUnhandled program-level node kinds:");
        for k in &unhandled {
            println!("  {k}");
        }
        println!("\nAll node kinds in tree:");
        for k in &all_kinds {
            println!("  {k}");
        }
    }
    // This test is purely diagnostic — never fails.
}
