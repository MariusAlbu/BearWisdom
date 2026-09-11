use super::*;
use crate::languages::csharp::profile::CSHARP_PROFILE;
use crate::type_checker::profile::default_profile::DEFAULT_PROFILE;
use crate::types::{ExtractedSymbol, SymbolKind};

/// Minimal scope config for C#-style tests.
const CSHARP_CONFIG: &[ScopeKind] = &[
    ScopeKind {
        node_kind: "namespace_declaration",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "class_declaration",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "method_declaration",
        name_field: "name",
    },
];

const CPP_CONFIG: &[ScopeKind] = &[
    ScopeKind {
        node_kind: "namespace_definition",
        name_field: "name",
    },
    ScopeKind {
        node_kind: "class_specifier",
        name_field: "name",
    },
];

fn parse_csharp(source: &str) -> tree_sitter::Tree {
    let lang: tree_sitter::Language = tree_sitter_c_sharp::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    parser.parse(source, None).unwrap()
}

fn parse_cpp(source: &str) -> tree_sitter::Tree {
    let lang: tree_sitter::Language = tree_sitter_cpp::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    parser.parse(source, None).unwrap()
}

#[test]
fn build_scopes_for_namespace_class_method() {
    let source = "namespace Foo { class Bar { void Baz() {} } }";
    let tree = parse_csharp(source);
    let scopes = build(
        tree.root_node(),
        source.as_bytes(),
        CSHARP_CONFIG,
        &CSHARP_PROFILE,
    );

    let names: Vec<&str> = scopes.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Foo"), "Missing Foo:  {names:?}");
    assert!(names.contains(&"Bar"), "Missing Bar:  {names:?}");
    assert!(names.contains(&"Baz"), "Missing Baz:  {names:?}");
}

#[test]
fn qualified_names_are_dotted() {
    let source = "namespace A { class B { void C() {} } }";
    let tree = parse_csharp(source);
    let scopes = build(
        tree.root_node(),
        source.as_bytes(),
        CSHARP_CONFIG,
        &CSHARP_PROFILE,
    );

    let c = scopes.iter().find(|s| s.name == "C").unwrap();
    assert_eq!(c.qualified_name, "A.B.C");
}

#[test]
fn find_scope_at_returns_deepest_scope() {
    let source = "namespace A { class B { void C() {} } }";
    let tree = parse_csharp(source);
    let scopes = build(
        tree.root_node(),
        source.as_bytes(),
        CSHARP_CONFIG,
        &CSHARP_PROFILE,
    );

    // Pick an offset inside the method body.
    // `void C()` starts somewhere after the opening brace of B.
    // `{` of C() body is what we're after — just pick end of the string.
    let inside_c_offset = source.find("void C").unwrap() + 5;
    let scope = find_scope_at(&scopes, inside_c_offset).unwrap();
    assert_eq!(
        scope.name, "C",
        "Expected deepest scope 'C', got '{}'",
        scope.name
    );
}

#[test]
fn qualify_helper_builds_full_name() {
    let entry = ScopeEntry {
        name: "Bar".to_string(),
        qualified_name: "Foo.Bar".to_string(),
        node_kind: "class_declaration",
        start_byte: 0,
        end_byte: 100,
        depth: 1,
    };
    let qname = qualify(&CSHARP_PROFILE, "GetById", Some(&entry));
    assert_eq!(qname, "Foo.Bar.GetById");
}

#[test]
fn qualify_with_no_scope_returns_bare_name() {
    let qname = qualify(&CSHARP_PROFILE, "GlobalFunc", None);
    assert_eq!(qname, "GlobalFunc");
}

#[test]
fn deeply_nested_ast_does_not_overflow() {
    // A pathologically deep but scope-free subtree: 50k nested parens nest
    // one CST node per level, the same shape a generated `.d.ts` produces for
    // a `type X = 'a' | 'b' | …` union with tens of thousands of members. A
    // recursive walk overflows the thread stack on this; the iterative walk
    // must build the (shallow) scope set without crashing and still qualify
    // the enclosing scopes correctly.
    let depth = 50_000;
    let expr = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    let source = format!("namespace N {{ class C {{ void M() {{ var x = {expr}; }} }} }}");
    let tree = parse_csharp(&source);
    let scopes = build(
        tree.root_node(),
        source.as_bytes(),
        CSHARP_CONFIG,
        &CSHARP_PROFILE,
    );

    let m = scopes
        .iter()
        .find(|s| s.name == "M")
        .expect("enclosing method scope must survive deep nesting");
    assert_eq!(m.qualified_name, "N.C.M");
}

#[test]
fn source_qualified_scope_uses_canonical_index_qname() {
    let source = "namespace pkg::inner { class Thing {}; }";
    let tree = parse_cpp(source);
    let profile = LanguageProfile {
        qname_separator: "::",
        ..DEFAULT_PROFILE
    };

    let scopes = build(tree.root_node(), source.as_bytes(), CPP_CONFIG, &profile);
    let thing = scopes.iter().find(|scope| scope.name == "Thing").unwrap();
    assert_eq!(thing.qualified_name, "pkg.inner.Thing");
    assert_eq!(
        qualify(&profile, "Member", Some(thing)),
        "pkg.inner.Thing.Member"
    );
}

#[test]
fn hoisted_source_package_prefixes_canonical_qnames() {
    let profile = LanguageProfile {
        qname_separator: "::",
        ..DEFAULT_PROFILE
    };
    let mut symbols = vec![
        symbol("Top", "Top", None),
        symbol("Nested", "Top.Nested", Some("Top")),
    ];

    prefix_top_level_qnames(&mut symbols, Some("pkg::api"), &profile);

    assert_eq!(symbols[0].qualified_name, "pkg.api.Top");
    assert_eq!(symbols[0].scope_path.as_deref(), Some("pkg.api"));
    assert_eq!(symbols[1].qualified_name, "pkg.api.Top.Nested");
    assert_eq!(symbols[1].scope_path.as_deref(), Some("pkg.api.Top"));
}

fn symbol(name: &str, qualified_name: &str, scope_path: Option<&str>) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind: SymbolKind::Class,
        visibility: None,
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope_path.map(str::to_string),
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}
