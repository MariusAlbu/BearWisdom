use super::extract;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use std::collections::BTreeSet;

fn sym(source: &str) -> Vec<ExtractedSymbol> { extract::extract(source, false).symbols }
fn refs(source: &str) -> Vec<ExtractedRef>    { extract::extract(source, false).refs }

#[test]
fn namespace_import_qualified_typeref_routes_to_module() {
    // `import * as Oazapfts from "@oazapfts/runtime";` followed by
    // `Oazapfts.RequestOpts` as a parameter type produces a TypeRef
    // emitted by `nested_type_identifier` as a single qualified string.
    // The post-pass `annotate_namespace_type_refs` must split the prefix
    // off and set `module` so demand-seed can pull the type's defining
    // file and the resolver can match it.
    let src = r#"
import * as Oazapfts from "@oazapfts/runtime";
export function call(opts?: Oazapfts.RequestOpts): void {}
"#;
    let r = extract::extract(src, false);
    let ns_ref = r
        .refs
        .iter()
        .find(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "RequestOpts")
        .expect("expected TypeRef target_name='RequestOpts' after rewrite");
    assert_eq!(
        ns_ref.module.as_deref(),
        Some("@oazapfts/runtime"),
        "expected module set to the import path"
    );
}

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
fn qualified_call_ref_gets_module_from_import() {
    // `UserService.findOne(id)` — the call ref should have module="./user.service"
    // because `UserService` is imported from that module.
    let src = r#"
import { UserService } from './user.service';
class Controller {
    async get(id: string) {
        return UserService.findOne(id);
    }
}
"#;
    let r = refs(src);
    let call = r.iter().find(|r| r.kind == EdgeKind::Calls && r.target_name == "findOne");
    assert!(call.is_some(), "no Calls ref for findOne; refs: {r:?}");
    assert_eq!(
        call.unwrap().module,
        Some("./user.service".to_string()),
        "module should be set from import; refs: {r:?}"
    );
}

#[test]
fn bare_call_does_not_get_spurious_module() {
    // `fetchData()` is a bare call with no object prefix — module must stay None.
    let src = r#"
import { UserService } from './user.service';
function run() { fetchData(); }
"#;
    let r = refs(src);
    let call = r.iter().find(|r| r.kind == EdgeKind::Calls && r.target_name == "fetchData");
    assert!(call.is_some(), "no Calls ref for fetchData; refs: {r:?}");
    assert_eq!(call.unwrap().module, None, "bare call should not get a module");
}

#[test]
fn aliased_import_call_ref_gets_module() {
    // `import { UserService as US } from './user.service'` — alias `US` is used
    // in the chain, so the module should resolve via the alias.
    let src = r#"
import { UserService as US } from './user.service';
class Controller {
    get(id: string) { return US.findOne(id); }
}
"#;
    let r = refs(src);
    let call = r.iter().find(|r| r.kind == EdgeKind::Calls && r.target_name == "findOne");
    assert!(call.is_some(), "no Calls ref for findOne; refs: {r:?}");
    assert_eq!(
        call.unwrap().module,
        Some("./user.service".to_string()),
        "aliased import should resolve correctly; refs: {r:?}"
    );
}

#[test]
fn namespace_import_call_ref_gets_module() {
    // `import * as svc from './service'` — `svc.doWork()` should get module="./service".
    let src = r#"
import * as svc from './service';
function run() { svc.doWork(); }
"#;
    let r = refs(src);
    let call = r.iter().find(|r| r.kind == EdgeKind::Calls && r.target_name == "doWork");
    assert!(call.is_some(), "no Calls ref for doWork; refs: {r:?}");
    assert_eq!(
        call.unwrap().module,
        Some("./service".to_string()),
        "namespace import should resolve correctly; refs: {r:?}"
    );
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

#[test]
fn inline_object_type_in_function_param() {
    // property_signature inside an inline object type in a function parameter.
    let r = extract::extract("function foo(opts: { x: number; y: string }) {}", false);
    assert!(
        r.symbols.iter().any(|s| s.name == "x" && s.kind == SymbolKind::Property),
        "expected Property 'x' from inline object type in function param; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "y" && s.kind == SymbolKind::Property),
        "expected Property 'y' from inline object type in function param; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn inline_object_type_in_var_annotation() {
    // property_signature inside an inline object type in a variable annotation.
    let r = extract::extract("const config: { host: string; port: number } = {} as any;", false);
    assert!(
        r.symbols.iter().any(|s| s.name == "host" && s.kind == SymbolKind::Property),
        "expected Property 'host' from inline object type in var annotation; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn inline_object_type_in_method_param() {
    // property_signature inside an inline object type in a method signature parameter.
    let r = extract::extract("interface IRepo { find(opts: { id: number }): User; }", false);
    assert!(
        r.symbols.iter().any(|s| s.name == "id" && s.kind == SymbolKind::Property),
        "expected Property 'id' from inline object type in method_signature param; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn inline_object_type_in_method_def_param() {
    // property_signature inside an inline object type in a method_definition parameter.
    let r = extract::extract("class Svc { handle(opts: { x: number }): void {} }", false);
    assert!(
        r.symbols.iter().any(|s| s.name == "x" && s.kind == SymbolKind::Property),
        "expected Property 'x' from inline object type in method_definition param; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
#[ignore]
fn debug_prisma_method_signature() {
    // Prisma-style interface with property_signature containing object_type with method_signatures
    let src = r#"
interface PrismaClient {
    user: {
        findUnique(args: { where: { id: number } }): Promise<User | null>;
        findMany(args?: { where?: UserWhereInput }): Promise<User[]>;
    };
    booking: {
        findUnique(args: { where: { id: number } }): Promise<Booking | null>;
        create(args: { data: BookingCreateInput }): Promise<Booking>;
    };
}
"#;
    let r = extract::extract(src, false);
    eprintln!("Symbols:");
    for s in &r.symbols {
        eprintln!("  {:?} {:?} line={}", s.kind, s.name, s.start_line);
    }
    let method_sigs: Vec<_> = r.symbols.iter().filter(|s| s.kind == SymbolKind::Method).collect();
    eprintln!("Method symbols: {}", method_sigs.len());
}

#[test]
#[ignore]
fn debug_type_alias_method_signature() {
    // Type alias with method_signatures in object type
    let src = r#"
type UserDelegate = {
    findUnique(args: FindUniqueArgs): User | null;
    findMany(args?: FindManyArgs): User[];
    create(args: CreateArgs): User;
};
"#;
    let r = extract::extract(src, false);
    eprintln!("Symbols:");
    for s in &r.symbols {
        eprintln!("  {:?} {:?} line={}", s.kind, s.name, s.start_line);
    }
}

// ---------------------------------------------------------------------------
// Re-export extraction tests
// ---------------------------------------------------------------------------

#[test]
fn reexport_named_emits_imports_ref() {
    // export { UserService } from './user.service'
    let result = extract::extract("export { UserService } from './user.service';", false);
    let r = result.refs.iter().find(|r| {
        r.kind == EdgeKind::Imports
            && r.target_name == "UserService"
            && r.module.as_deref() == Some("./user.service")
    });
    assert!(r.is_some(), "named re-export should emit Imports ref with module set");
}

#[test]
fn reexport_named_alias_emits_original_name() {
    // export { Foo as Bar } from './foo'
    // Extractor should emit the *original* name ("Foo"), not the alias.
    let result = extract::extract("export { Foo as Bar } from './foo';", false);
    let r = result.refs.iter().find(|r| {
        r.kind == EdgeKind::Imports
            && r.target_name == "Foo"
            && r.module.as_deref() == Some("./foo")
    });
    assert!(r.is_some(), "aliased re-export should emit Imports ref with original name");
    // Alias ("Bar") should NOT appear as a target_name with module set.
    let alias_ref = result.refs.iter().find(|r| {
        r.kind == EdgeKind::Imports && r.target_name == "Bar"
    });
    assert!(alias_ref.is_none(), "alias name should not appear as Imports ref target");
}

#[test]
fn reexport_star_emits_wildcard_ref() {
    // export * from './utils'
    let result = extract::extract("export * from './utils';", false);
    let r = result.refs.iter().find(|r| {
        r.kind == EdgeKind::Imports
            && r.target_name == "*"
            && r.module.as_deref() == Some("./utils")
    });
    assert!(r.is_some(), "export * should emit Imports ref with target_name='*'");
}

#[test]
fn reexport_star_as_emits_wildcard_ref() {
    // export * as ns from './utils'
    let result = extract::extract("export * as ns from './utils';", false);
    let r = result.refs.iter().find(|r| {
        r.kind == EdgeKind::Imports
            && r.target_name == "*"
            && r.module.as_deref() == Some("./utils")
    });
    assert!(r.is_some(), "export * as ns should emit Imports ref with target_name='*'");
}

#[test]
fn reexport_without_from_emits_no_imports_ref() {
    // export { Foo } — no `from`, so this is a re-export of a local symbol.
    // Should NOT emit an Imports ref (no module to follow).
    let result = extract::extract("const Foo = 1; export { Foo };", false);
    let r = result.refs.iter().find(|r| {
        r.kind == EdgeKind::Imports && r.target_name == "Foo"
    });
    assert!(r.is_none(), "export without from should not emit Imports ref");
}

#[test]
fn reexport_multiple_named_emits_one_ref_per_export() {
    // export { A, B, C } from './mod'
    let result = extract::extract("export { A, B, C } from './mod';", false);
    let import_refs: Vec<_> = result.refs.iter()
        .filter(|r| r.kind == EdgeKind::Imports && r.module.as_deref() == Some("./mod"))
        .collect();
    let names: Vec<&str> = import_refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"A"), "A should be in import refs");
    assert!(names.contains(&"B"), "B should be in import refs");
    assert!(names.contains(&"C"), "C should be in import refs");
    assert_eq!(import_refs.len(), 3, "should emit one Imports ref per export specifier");
}

// ---------------------------------------------------------------------------
// Local variable type inference from new_expression
// ---------------------------------------------------------------------------

#[test]
fn new_expression_emits_typeref_for_variable() {
    // `const service = new UserService()` should emit TypeRef "UserService"
    // attached to the `service` Variable symbol (chain: None).
    let src = "const service = new UserService();";
    let r = extract::extract(src, false);

    let svc_sym = r.symbols.iter().enumerate().find(|(_, s)| s.name == "service");
    assert!(svc_sym.is_some(), "Expected Variable symbol 'service'");
    let (svc_idx, _) = svc_sym.unwrap();

    let typeref = r.refs.iter().find(|rf| {
        rf.source_symbol_index == svc_idx
            && rf.kind == EdgeKind::TypeRef
            && rf.target_name == "UserService"
            && rf.chain.is_none()
            && rf.module.is_none()
    });
    assert!(
        typeref.is_some(),
        "Expected TypeRef 'UserService' from 'service'; refs = {:?}",
        r.refs
            .iter()
            .filter(|rf| rf.source_symbol_index == svc_idx)
            .collect::<Vec<_>>()
    );
}

#[test]
fn explicit_type_annotation_takes_priority_over_new_expression() {
    // `const service: UserService = new UserService()` — explicit annotation
    // should also produce a TypeRef.
    let src = "const service: UserService = new UserService();";
    let r = extract::extract(src, false);

    let svc_sym = r.symbols.iter().enumerate().find(|(_, s)| s.name == "service");
    assert!(svc_sym.is_some(), "Expected Variable symbol 'service'");
    let (svc_idx, _) = svc_sym.unwrap();

    // At least one TypeRef pointing at UserService from the variable.
    let has_typeref = r.refs.iter().any(|rf| {
        rf.source_symbol_index == svc_idx
            && rf.kind == EdgeKind::TypeRef
            && rf.target_name == "UserService"
    });
    assert!(has_typeref, "Expected at least one TypeRef 'UserService' from 'service'");
}

// ---------------------------------------------------------------------------
// jest/vitest assertion chain refs
// ---------------------------------------------------------------------------

/// Pattern 1 — `expect(x).toEqual(y)`:
/// The `toEqual` Calls ref must carry a chain with `expect` as the root segment.
/// The chain walker needs this to find `toEqual` on the return type of `expect`.
#[test]
fn jest_expect_to_equal_has_chain_ref() {
    let src = "function t() { expect(x).toEqual(y); }";
    let r = refs(src);

    let to_equal = r.iter().find(|r| r.kind == EdgeKind::Calls && r.target_name == "toEqual");
    assert!(to_equal.is_some(), "no Calls ref for toEqual; refs: {r:?}");

    let chain = to_equal.unwrap().chain.as_ref()
        .expect("toEqual ref must have a chain (chain walker needs it)");

    // Chain must be [expect, toEqual] — root is `expect`, last is `toEqual`.
    assert_eq!(chain.segments.len(), 2, "chain should have 2 segments: [expect, toEqual]; got: {chain:?}");
    assert_eq!(chain.segments[0].name, "expect", "chain root should be 'expect'");
    assert_eq!(chain.segments[1].name, "toEqual", "chain last should be 'toEqual'");
}

/// Pattern 2 — `expect(x).to.equal(y)` (chai BDD):
/// The `equal` Calls ref must have a 3-segment chain: [expect, to, equal].
/// The intermediate `.to` property access is Tier 1.5 chain-walking state.
#[test]
fn chai_bdd_expect_to_equal_has_chain_ref() {
    let src = "function t() { expect(x).to.equal(y); }";
    let r = refs(src);

    let equal = r.iter().find(|r| r.kind == EdgeKind::Calls && r.target_name == "equal");
    assert!(equal.is_some(), "no Calls ref for equal; refs: {r:?}");

    let chain = equal.unwrap().chain.as_ref()
        .expect("equal ref must have a chain for chai BDD pattern");

    // Chain must be [expect, to, equal].
    assert_eq!(chain.segments.len(), 3, "chain should have 3 segments: [expect, to, equal]; got: {chain:?}");
    assert_eq!(chain.segments[0].name, "expect", "chain[0] should be 'expect'");
    assert_eq!(chain.segments[1].name, "to",     "chain[1] should be 'to'");
    assert_eq!(chain.segments[2].name, "equal",  "chain[2] should be 'equal'");
}

/// Pattern 3 — `vi.fn().mockResolvedValue(42)` (vitest spy):
/// The `mockResolvedValue` Calls ref must carry a 3-segment chain: [vi, fn, mockResolvedValue].
#[test]
fn vitest_spy_mock_resolved_value_has_chain_ref() {
    let src = "function t() { vi.fn().mockResolvedValue(42); }";
    let r = refs(src);

    let mrv = r.iter().find(|r| r.kind == EdgeKind::Calls && r.target_name == "mockResolvedValue");
    assert!(mrv.is_some(), "no Calls ref for mockResolvedValue; refs: {r:?}");

    let chain = mrv.unwrap().chain.as_ref()
        .expect("mockResolvedValue ref must have a chain");

    // Chain must be [vi, fn, mockResolvedValue].
    assert_eq!(chain.segments.len(), 3,
        "chain should have 3 segments: [vi, fn, mockResolvedValue]; got: {chain:?}");
    assert_eq!(chain.segments[0].name, "vi",                  "chain[0] should be 'vi'");
    assert_eq!(chain.segments[1].name, "fn",                  "chain[1] should be 'fn'");
    assert_eq!(chain.segments[2].name, "mockResolvedValue",   "chain[2] should be 'mockResolvedValue'");
}

#[test]
fn type_parameter_usage_inside_declaration_is_not_emitted_as_ref() {
    // `Target` is a type parameter of `TargetedEvent`; its use inside the
    // type alias body must NOT surface as an unresolved external ref.
    let src = r#"
        export type TargetedEvent<Target extends EventTarget = EventTarget> = {
            readonly currentTarget: Target;
        };
    "#;
    let r = refs(src);
    let leaked: Vec<_> = r
        .iter()
        .filter(|r| r.target_name == "Target" && r.kind == EdgeKind::TypeRef)
        .collect();
    assert!(
        leaked.is_empty(),
        "type-param `Target` leaked as ref: {leaked:?}"
    );
    // And the constraint `EventTarget` still emits (it's genuinely external).
    assert!(
        r.iter().any(|r| r.target_name == "EventTarget" && r.kind == EdgeKind::TypeRef),
        "EventTarget constraint should still emit a ref; refs: {r:?}"
    );
}

#[test]
fn type_parameter_scope_does_not_suppress_same_named_ref_outside() {
    // `T` is a type parameter of `Box<T>`; outside Box, `T` is not in scope
    // and must still emit as a ref.
    let src = r#"
        export interface Box<T> { value: T }
        export type UseT = T;
    "#;
    let r = refs(src);
    // The outer `T` (in UseT = T) is outside any scope and should emit.
    let outer_t = r
        .iter()
        .filter(|r| r.target_name == "T" && r.kind == EdgeKind::TypeRef)
        .count();
    assert!(
        outer_t >= 1,
        "out-of-scope `T` should still emit a ref; refs: {r:?}"
    );
}

#[test]
fn function_type_parameter_suppressed_inside_function_body_annotations() {
    let src = r#"
        export function identity<Target>(x: Target): Target { return x; }
    "#;
    let r = refs(src);
    let leaked: Vec<_> = r
        .iter()
        .filter(|r| r.target_name == "Target" && r.kind == EdgeKind::TypeRef)
        .collect();
    assert!(
        leaked.is_empty(),
        "type-param `Target` in function signature leaked: {leaked:?}"
    );
}
