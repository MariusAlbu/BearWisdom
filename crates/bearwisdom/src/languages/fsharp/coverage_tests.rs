// =============================================================================
// fsharp/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
//
// Grammar notes (tree-sitter-fsharp):
// - A lone `let` inside a `module` parses as an ERROR node unless at least
//   one other declaration follows it.
// - Value bindings (`let x = 42`) produce `value_declaration_left` whose name
//   is in a nested `identifier_pattern → long_identifier_or_op` chain.
//   The extractor walks this chain via `extract_value_decl_name`.
// - `type_definition` children of `named_module` are not currently extracted
//   due to a traversal gap in `extract_type_def` when called from `visit`.
//
// Additional symbol_node_kinds (rules.md):
//   type_definition → Class (anon_type_defn), Interface (interface_type_defn),
//                     TypeAlias (type_abbrev_defn)
//   union_type_case → EnumMember, enum_type_case → EnumMember (TODO)
//   record_field → Field (TODO), module_abbrev → TypeAlias (TODO)
//   exception_definition → Struct (TODO)
//
// Additional ref_node_kinds:
//   interface_implementation → Implements (TODO)
//   class_inherits_decl → Inherits (TODO)
// =============================================================================

use super::extract::extract;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, LanguageResolver, RefContext};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

// ---------------------------------------------------------------------------
// Module-qname propagation (regression: pre-fix every symbol got an unprefixed
// qname, so `let bind` inside `module Async = ...` was indexed as "bind"
// rather than "Async.bind", and dotted refs like `Async.bind` couldn't resolve.)
// ---------------------------------------------------------------------------

#[test]
fn function_inside_module_qualified_with_module_name() {
    let r = extract("module Async\nlet bind f = f\nlet other = 0");
    let bind = r.symbols.iter().find(|s| s.name == "bind").expect("bind");
    assert_eq!(bind.qualified_name, "Async.bind");
    assert_eq!(bind.scope_path.as_deref(), Some("Async"));
}

#[test]
fn type_inside_module_qualified_with_module_name() {
    let r = extract("module Foo\nlet x = 0\ntype Person = { Name: string }");
    let p = r.symbols.iter().find(|s| s.name == "Person").expect("Person");
    assert_eq!(p.qualified_name, "Foo.Person");
}

#[test]
fn nested_module_qname_chain() {
    let r = extract("module Outer\n\nmodule Inner =\n    let value = 42\n    let other = 0\n");
    let v = r
        .symbols
        .iter()
        .find(|s| s.name == "value")
        .unwrap_or_else(|| {
            panic!(
                "expected `value` symbol; got {:?}",
                r.symbols.iter().map(|s| (&s.name, s.kind, &s.qualified_name)).collect::<Vec<_>>()
            )
        });
    assert_eq!(v.qualified_name, "Outer.Inner.value");
}

#[test]
fn record_field_qualified_with_record_qname() {
    let r = extract("module Foo\nlet x = 0\ntype Person = { Name: string }");
    let f = r.symbols.iter().find(|s| s.name == "Name" && s.kind == SymbolKind::Field).expect("Name");
    assert_eq!(f.qualified_name, "Foo.Person.Name");
}

#[test]
fn union_case_qualified_with_union_qname() {
    let r = extract(
        "module Foo\nlet x = 0\ntype Shape =\n    | Circle of float\n    | Square of float",
    );
    let c = r
        .symbols
        .iter()
        .find(|s| s.name == "Circle" && s.kind == SymbolKind::EnumMember)
        .expect("Circle");
    assert_eq!(c.qualified_name, "Foo.Shape.Circle");
}

#[test]
fn top_level_let_without_module_unprefixed() {
    let r = extract("let x = 1\nlet y = 2");
    let x = r.symbols.iter().find(|s| s.name == "x").expect("x");
    assert_eq!(x.qualified_name, "x");
    assert_eq!(x.scope_path, None);
}

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `function_or_value_defn`  →  Function (has parameters)
/// Requires a second declaration to avoid lone-let grammar error.
#[test]
fn symbol_function_or_value_defn_function() {
    // Two function bindings: grammar parses cleanly; both should extract.
    let r = extract("module MyModule\nlet foo x = x + 1\nlet bar y = y * 2");
    assert!(
        r.symbols.iter().any(|s| s.name == "foo" && s.kind == SymbolKind::Function),
        "expected Function foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `function_or_value_defn`  →  Variable (no parameters)
/// Value bindings (`let x = 42`) produce `value_declaration_left` with the name
/// nested under `identifier_pattern → long_identifier_or_op`.
#[test]
fn symbol_function_or_value_defn_variable() {
    let r = extract("module MyModule\nlet answer = 42\nlet other = 0");
    assert!(
        r.symbols.iter().any(|s| s.name == "answer" && s.kind == SymbolKind::Variable),
        "expected Variable 'answer' from value binding; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `type_definition`  →  Struct (record type)
#[test]
fn symbol_type_definition_record() {
    let r = extract("module MyModule\nlet foo x = x + 1\ntype Person = { Name: string }");
    assert!(
        r.symbols.iter().any(|s| s.name == "Person" && s.kind == SymbolKind::Struct),
        "expected Struct Person from record_type_defn; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `type_definition`  →  Enum (discriminated union)
#[test]
fn symbol_type_definition_union() {
    let r = extract("module MyModule\nlet foo x = x\ntype Shape =\n    | Circle of float\n    | Square of float");
    assert!(
        r.symbols.iter().any(|s| s.name == "Shape" && s.kind == SymbolKind::Enum),
        "expected Enum Shape from union_type_defn; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `module_defn`  →  Namespace
#[test]
fn symbol_module_defn() {
    let r = extract("module MyModule\nlet foo x = x + 1\nlet bar y = y * 2");
    assert!(
        r.symbols.iter().any(|s| s.name == "MyModule" && s.kind == SymbolKind::Namespace),
        "expected Namespace MyModule; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `named_module`  —  file-level `module A.B` declaration
#[test]
fn symbol_named_module() {
    let r = extract("module MyApp.Core\nlet init x = x\nlet cleanup x = x");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace),
        "expected Namespace from named_module; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `namespace`  →  Namespace
/// A `namespace` declaration followed by a single binding parses as ERROR.
/// This test uses no inner declarations and just verifies no panic.
#[test]
fn symbol_namespace() {
    let r = extract("namespace MyApp.Domain\nmodule Core =\n    let x = 1");
    // Namespace extraction from `namespace` keyword depends on grammar parse quality.
    let _ = r;
}

/// symbol_node_kind: `import_decl`  —  listed in both symbol_node_kinds and ref_node_kinds.
/// `open` declarations produce an Imports ref.
#[test]
fn symbol_import_decl() {
    let r = extract("module M\nopen System.Collections.Generic\nlet foo x = x");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from import_decl; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `application_expression`  →  Calls edge
/// `String.length x` is an application_expression inside a function body.
/// A second declaration is needed for the grammar to parse correctly.
#[test]
fn ref_application_expression() {
    let r = extract("module M\nlet bar x = String.length x\nlet dummy y = y");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "String.length" && rf.kind == EdgeKind::Calls),
        "expected Calls String.length from application_expression; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `dot_expression`  —  member access (e.g., `s.Length`)
/// The extractor recurses through dot_expression; no edge is emitted but no panic.
#[test]
fn ref_dot_expression() {
    let r = extract("module M\nlet foo s = s.Length\nlet bar y = y");
    assert!(
        r.symbols.iter().any(|s| s.name == "foo"),
        "expected Function foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `import_decl`  →  Imports edge
#[test]
fn ref_import_decl() {
    let r = extract("module M\nopen System.IO\nlet foo x = x");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "System.IO"),
        "expected Imports System.IO; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional type_definition variants
// ---------------------------------------------------------------------------

/// symbol_node_kind: `type_definition` → Class (anon_type_defn)
/// Probe test: does the traversal gap actually affect extraction?
#[test]
fn symbol_type_definition_class_does_not_crash() {
    let r = extract("module MyModule\nlet foo x = x + 1\ntype MyClass() =\n    member this.Value = 42");
    let _ = r;
    // eprintln to see actual symbols — uncomment to debug:
    // eprintln!("symbols: {:?}", r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>());
}

/// `type MyClass()` inside a `named_module` → Class symbol extracted.
/// Use the type as the FIRST declaration so it is not swallowed by a `let` continuation.
#[test]
fn symbol_type_definition_class_in_named_module() {
    let r = extract("module MyModule\ntype MyClass() =\n    member this.Value = 42\nlet foo x = x + 1");
    assert!(
        r.symbols.iter().any(|s| s.name == "MyClass" && s.kind == SymbolKind::Class),
        "expected Class 'MyClass' inside named_module; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `type_definition` → Interface (interface_type_defn)
#[test]
fn symbol_type_definition_interface() {
    let r = extract(
        "module MyModule\nlet foo x = x + 1\ntype IAnimal =\n    interface\n        abstract member Speak: unit -> string\n    end",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "IAnimal" && s.kind == SymbolKind::Interface),
        "expected Interface 'IAnimal' from interface_type_defn; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `type_definition` → TypeAlias (type_abbrev_defn)
/// `type Name = string` parses ambiguously (treated as union_type_defn in tree-sitter-fsharp).
/// A qualified RHS forces `type_abbrev_defn`: `type Name = System.String`.
/// The type MUST appear before any `let` binding to avoid being swallowed as its continuation.
#[test]
fn symbol_type_definition_type_alias_emits_type_alias() {
    let r = extract("module MyModule\ntype Name = System.String\nlet foo x = x + 1");
    assert!(
        r.symbols.iter().any(|s| s.name == "Name" && s.kind == SymbolKind::TypeAlias),
        "expected TypeAlias 'Name' from type_abbrev_defn; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `union_type_case` → SymbolKind::EnumMember
#[test]
fn symbol_union_type_case() {
    let r = extract(
        "module MyModule\nlet foo x = x\ntype Color =\n    | Red\n    | Green\n    | Blue",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Color" && s.kind == SymbolKind::Enum),
        "expected Enum 'Color'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Red" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Red'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Green" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Green'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Blue" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Blue'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `enum_type_case` → SymbolKind::EnumMember
#[test]
fn symbol_enum_type_case() {
    let r = extract(
        "module MyModule\nlet foo x = x\ntype Status =\n    | Active = 1\n    | Inactive = 0",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Status" && s.kind == SymbolKind::Enum),
        "expected Enum 'Status'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Active" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Active'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Inactive" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Inactive'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `record_field` → SymbolKind::Field
#[test]
fn symbol_record_field() {
    let r = extract(
        "module MyModule\nlet foo x = x\ntype Point = { X: float; Y: float }",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Point" && s.kind == SymbolKind::Struct),
        "expected Struct 'Point'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "X" && s.kind == SymbolKind::Field),
        "expected Field 'X'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Y" && s.kind == SymbolKind::Field),
        "expected Field 'Y'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `module_abbrev` → SymbolKind::TypeAlias
#[test]
fn symbol_module_abbrev() {
    let r = extract("module MyModule\nmodule L = System.Collections.Generic.List\nlet foo x = x");
    assert!(
        r.symbols.iter().any(|s| s.name == "L" && s.kind == SymbolKind::TypeAlias),
        "expected TypeAlias 'L' from module_abbrev; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `exception_definition` → SymbolKind::Struct
#[test]
fn symbol_exception_definition() {
    let r = extract(
        "module MyModule\nlet foo x = x\nexception MyError of string",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "MyError" && s.kind == SymbolKind::Struct),
        "expected Struct 'MyError' from exception_definition; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// #r directive extraction (F# script files)
// ---------------------------------------------------------------------------

/// `open` statements after `#if`/`#endif` conditional blocks may not be
/// recognized by tree-sitter-fsharp. The `#r` directive pre-pass handles
/// DLL imports from the header section to ensure external namespaces can still
/// be inferred for refs in such files.
#[test]
fn ref_import_decl_after_conditional_block_may_not_parse() {
    // The tree-sitter-fsharp grammar does not parse preprocessor conditionals
    // (#if/#endif). After such a block, `open` declarations may not be
    // emitted as import_decl nodes. This test documents the known behavior
    // and the `#r` directive pre-pass is the mitigation for such files.
    let src = "#if !FORNAX\n#load \"./foo.fsx\"\n#endif\n\nopen System\nlet x = 1";
    let r = extract(src);
    // Either zero or one Imports ref for System — both are acceptable outcomes
    // depending on the tree-sitter-fsharp version's handling of #if blocks.
    let _ = r; // document-only test
}

/// `#r "path/Fornax.Core.dll"` → Imports ref with target `Fornax.Core`.
#[test]
fn hash_r_directive_emits_imports_ref() {
    let src = "#r \"../../packages/Fornax.Core.dll\"\n#r \"Giraffe.dll\"\n\nopen Html\n\nlet x = 1";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "Fornax.Core"),
        "expected Imports Fornax.Core from #r directive; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "Giraffe"),
        "expected Imports Giraffe from #r directive; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// `#r`-derived imports in `file_ctx` cause bare-name Calls refs from the
/// same file to be classified as external by `infer_external_namespace`.
#[test]
fn infer_external_namespace_from_hash_r_import() {
    use super::resolve::FSharpResolver;
    use crate::types::ExtractedRef;

    let resolver = FSharpResolver;

    // Simulate a ParsedFile that has a #r-derived Imports ref for Fornax.Core.
    // build_file_context converts this to a FileContext with one import entry.
    let fornax_import = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Fornax.Core".to_string(),
        kind: EdgeKind::Imports,
        line: 0,
        module: Some("Fornax.Core".to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let file_ctx = FileContext {
        file_path: "docs/generators/apiref.fsx".to_string(),
        language: "fsharp".to_string(),
        imports: vec![ImportEntry {
            imported_name: "Fornax.Core".to_string(),
            module_path: Some("Fornax.Core".to_string()),
            alias: None,
            is_wildcard: true,
        }],
        file_namespace: None,
    };

    let div_ref = ExtractedRef {
        source_symbol_index: 0,
        target_name: "div".to_string(),
        kind: EdgeKind::Calls,
        line: 20,
        module: None,
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    };
    let dummy_symbol = ExtractedSymbol {
        name: "generate".to_string(),
        qualified_name: "generate".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 10,
        end_line: 30,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    };
    let ref_ctx = RefContext {
        extracted_ref: &div_ref,
        source_symbol: &dummy_symbol,
        scope_chain: vec![],
        file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert!(
        ns.is_some(),
        "expected Some namespace for 'div' with Fornax.Core import; got None"
    );
    assert_eq!(ns.as_deref(), Some("Fornax"),
        "expected 'Fornax' namespace; got {:?}", ns
    );
}

/// `#r` directives before a `let` binding are extracted; scanning stops after
/// the first non-header line to avoid false positives in the file body.
#[test]
fn hash_r_directive_stops_at_first_non_header_line() {
    // The `#r` after the `let` binding must NOT be extracted — it's in the body.
    let src = "#r \"Giraffe.dll\"\n\nlet x = 1\n#r \"Saturn.dll\"\n";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "Giraffe"),
        "expected Imports Giraffe; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert!(
        !r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "Saturn"),
        "Saturn #r after let binding must not be extracted; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref_node_kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `interface_implementation` → EdgeKind::Implements
#[test]
fn ref_interface_implementation() {
    let r = extract(concat!(
        "module MyModule\n",
        "let foo x = x\n",
        "type Dog() =\n",
        "    interface System.IDisposable with\n",
        "        member this.Dispose() = ()\n",
    ));
    assert!(
        r.symbols.iter().any(|s| s.name == "Dog"),
        "expected symbol 'Dog'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements),
        "expected Implements ref from interface_implementation; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `class_inherits_decl` → EdgeKind::Inherits
#[test]
fn ref_class_inherits_decl() {
    let r = extract(concat!(
        "module MyModule\n",
        "let foo x = x\n",
        "type Animal(name: string) =\n",
        "    member this.Name = name\n",
        "type Dog(name: string) =\n",
        "    inherit Animal(name)\n",
        "    member this.Bark() = \"woof\"\n",
    ));
    assert!(
        r.symbols.iter().any(|s| s.name == "Dog"),
        "expected symbol 'Dog'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "Animal"),
        "expected Inherits->Animal from class_inherits_decl; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

