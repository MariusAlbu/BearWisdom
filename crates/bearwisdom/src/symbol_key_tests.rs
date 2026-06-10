// =============================================================================
// symbol_key_tests.rs — stable-key formula + mergeable predicate
// =============================================================================

use super::*;
use crate::type_checker::core::types::{GenericParamId, Type, TypeArena, TypeId};
use crate::types::{SymbolKind, Visibility};
use std::num::NonZeroU32;

/// Build an `ExtractedSymbol` with just the key-relevant fields populated.
fn sym(
    qname: &str,
    kind: SymbolKind,
    param_types: Vec<TypeId>,
    generic_arity: usize,
    signature: Option<&str>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name: qname.rsplit('.').next().unwrap_or(qname).to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: signature.map(str::to_string),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types,
        // Only the count matters for the key; the ids themselves are unused.
        generic_params: (0..generic_arity)
            .map(|i| GenericParamId(NonZeroU32::new(i as u32 + 1).unwrap()))
            .collect(),
    }
}

// --- key core --------------------------------------------------------------

#[test]
fn overloads_differ_by_param_types() {
    let arena = TypeArena::new();
    let a = sym(
        "M.foo",
        SymbolKind::Method,
        vec![arena.intern_type_str("int")],
        0,
        None,
    );
    let b = sym(
        "M.foo",
        SymbolKind::Method,
        vec![arena.intern_type_str("string")],
        0,
        None,
    );
    assert_ne!(
        symbol_key("csharp", &a, 1, &arena),
        symbol_key("csharp", &b, 1, &arena),
        "foo(int) and foo(string) are distinct symbols"
    );
}

#[test]
fn body_change_keeps_key_stable() {
    // The key never sees a body — same name/kind/params/arity ⇒ same key,
    // regardless of an unrelated signature string difference.
    let arena = TypeArena::new();
    let a = sym(
        "M.foo",
        SymbolKind::Method,
        vec![arena.intern_type_str("int")],
        0,
        Some("v1"),
    );
    let b = sym(
        "M.foo",
        SymbolKind::Method,
        vec![arena.intern_type_str("int")],
        0,
        Some("v2"),
    );
    assert_eq!(
        symbol_key("csharp", &a, 1, &arena),
        symbol_key("csharp", &b, 1, &arena)
    );
}

#[test]
fn generic_arity_is_part_of_the_key() {
    let arena = TypeArena::new();
    let plain = sym("M.Foo", SymbolKind::Class, vec![], 0, None);
    let generic = sym("M.Foo", SymbolKind::Class, vec![], 1, None);
    assert_ne!(
        symbol_key("csharp", &plain, 1, &arena),
        symbol_key("csharp", &generic, 1, &arena),
        "Foo and Foo<T> are distinct"
    );
}

#[test]
fn untyped_params_become_underscore_and_preserve_arity() {
    let arena = TypeArena::new();
    let unknown = arena.intern(Type::Unknown);
    let two = sym("M.f", SymbolKind::Function, vec![unknown, unknown], 0, None);
    let one = sym("M.f", SymbolKind::Function, vec![unknown], 0, None);
    let k2 = symbol_key("javascript", &two, 1, &arena);
    assert!(k2.ends_with("#(_,_)"), "arity-2 untyped → (_,_): {k2}");
    assert_ne!(
        k2,
        symbol_key("javascript", &one, 1, &arena),
        "arity still disambiguates"
    );
}

#[test]
fn param_type_whitespace_is_normalized() {
    let arena = TypeArena::new();
    let spaced = sym(
        "M.g",
        SymbolKind::Method,
        vec![arena.intern_type_str("Map< string , int >")],
        0,
        None,
    );
    let tight = sym(
        "M.g",
        SymbolKind::Method,
        vec![arena.intern_type_str("Map<string,int>")],
        0,
        None,
    );
    assert_eq!(
        symbol_key("csharp", &spaced, 1, &arena),
        symbol_key("csharp", &tight, 1, &arena),
        "whitespace inside a type spelling is not significant"
    );
}

#[test]
fn non_overloadable_kinds_omit_params() {
    let arena = TypeArena::new();
    let field = sym("M.x", SymbolKind::Field, vec![], 0, None);
    let key = symbol_key("csharp", &field, 1, &arena);
    assert!(!key.contains('('), "fields carry no param list: {key}");
}

// --- mergeable scoping -----------------------------------------------------

#[test]
fn non_mergeable_symbol_is_file_scoped() {
    let arena = TypeArena::new();
    let m = sym("M.foo", SymbolKind::Method, vec![], 0, None);
    let in_f1 = symbol_key("csharp", &m, 1, &arena);
    let in_f2 = symbol_key("csharp", &m, 2, &arena);
    assert_ne!(
        in_f1, in_f2,
        "same private method in two files stays two symbols"
    );
    assert!(in_f1.starts_with("1:") && in_f2.starts_with("2:"));
}

#[test]
fn mergeable_symbol_is_file_independent() {
    let arena = TypeArena::new();
    let ns = sym("App.Models", SymbolKind::Namespace, vec![], 0, None);
    assert_eq!(
        symbol_key("csharp", &ns, 1, &arena),
        symbol_key("csharp", &ns, 2, &arena),
        "a namespace declared in two files is one logical symbol"
    );
}

#[test]
fn csharp_partial_class_merges_plain_class_does_not() {
    let arena = TypeArena::new();
    let partial = sym(
        "App.Foo",
        SymbolKind::Class,
        vec![],
        0,
        Some("public partial class Foo"),
    );
    let plain = sym(
        "App.Foo",
        SymbolKind::Class,
        vec![],
        0,
        Some("public class Foo"),
    );
    assert_eq!(
        symbol_key("csharp", &partial, 1, &arena),
        symbol_key("csharp", &partial, 2, &arena),
        "partial class merges across files"
    );
    assert_ne!(
        symbol_key("csharp", &plain, 1, &arena),
        symbol_key("csharp", &plain, 2, &arena),
        "non-partial class does not merge"
    );
}

// --- is_mergeable predicate ------------------------------------------------

#[test]
fn mergeable_predicate_seed() {
    use SymbolKind::*;
    // Universal.
    assert!(is_mergeable("rust", Namespace, None));
    assert!(is_mergeable("go", Module, None));
    // Per-language.
    assert!(is_mergeable("ruby", Class, None));
    assert!(is_mergeable("typescript", Interface, None));
    assert!(is_mergeable("csharp", Class, Some("partial class X")));
    // Negatives.
    assert!(!is_mergeable("csharp", Class, Some("class X")));
    assert!(!is_mergeable("java", Class, None));
    assert!(!is_mergeable("rust", Function, None));
}
