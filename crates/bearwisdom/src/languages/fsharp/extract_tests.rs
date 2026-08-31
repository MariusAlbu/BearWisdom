// =============================================================================
// fsharp/extract_tests.rs — module-header extraction + member-symbol emission
//
// Header shapes: plain and access-modifier-headed paths parse cleanly in the
// `name` field; a `rec` header mis-binds the `name` field to the keyword under
// error recovery and pushes the real dotted path into a following
// application_expression / dot_expression.
// Member emission: a `method_or_prop_defn` inside a type yields a
// Method/Property symbol parented to the enclosing type, so member lookups on
// internal receivers resolve against a real symbol row.
// =============================================================================

use super::extract::extract;
use crate::types::{ExtractionResult, SymbolKind};

/// The Namespace symbol a module header should yield, by exact name.
fn namespace<'a>(r: &'a ExtractionResult, name: &str) -> &'a crate::types::ExtractedSymbol {
    r.symbols
        .iter()
        .find(|s| s.kind == SymbolKind::Namespace && s.name == name)
        .unwrap_or_else(|| {
            panic!(
                "expected Namespace {name:?}; got {:?}",
                r.symbols
                    .iter()
                    .map(|s| (s.name.as_str(), s.kind))
                    .collect::<Vec<_>>()
            )
        })
}

/// Assert the let-binding `binding` carries the container-qualified qname.
fn assert_child_qualified(r: &ExtractionResult, binding: &str, container: &str) {
    let sym = r
        .symbols
        .iter()
        .find(|s| s.name == binding)
        .unwrap_or_else(|| panic!("expected binding {binding:?} to be extracted"));
    assert_eq!(
        sym.qualified_name,
        format!("{container}.{binding}"),
        "binding {binding:?} should be qualified under {container:?}"
    );
    assert_eq!(sym.scope_path.as_deref(), Some(container));
}

#[test]
fn modifier_headed_module_yields_full_dotted_namespace() {
    let r = extract("module internal A.B.C\n\nlet fixup x = x\n");
    let ns = namespace(&r, "A.B.C");
    assert_eq!(ns.qualified_name, "A.B.C");
    assert_child_qualified(&r, "fixup", "A.B.C");
}

#[test]
fn rec_module_recovers_full_dotted_namespace() {
    // `module rec A.B` parses under error recovery: the name field mis-binds
    // to the `rec` keyword and `A.B` lands in an application_expression.
    let r = extract("module rec A.B\n\nlet f x = x\n");
    let ns = namespace(&r, "A.B");
    assert_eq!(ns.qualified_name, "A.B");
    assert!(
        !r.symbols.iter().any(|s| s.name == "rec"),
        "the mis-bound `rec` keyword must not become a symbol"
    );
    assert_child_qualified(&r, "f", "A.B");
}

#[test]
fn rec_module_three_segment_path_recovers_via_dot_expression() {
    // Three-plus segments recover through the dot_expression shape.
    let r = extract("module rec Fable.AST.Fable\n\nlet f x = x\n");
    let ns = namespace(&r, "Fable.AST.Fable");
    assert_eq!(ns.qualified_name, "Fable.AST.Fable");
    assert_child_qualified(&r, "f", "Fable.AST.Fable");
}

#[test]
fn modifier_and_rec_module_recovers_full_dotted_namespace() {
    let r = extract("module internal rec A.B\n\nlet f x = x\n");
    let ns = namespace(&r, "A.B");
    assert_eq!(ns.qualified_name, "A.B");
    assert_child_qualified(&r, "f", "A.B");
}

#[test]
fn private_simple_module_yields_namespace() {
    let r = extract("module private M\n\nlet g y = y\n");
    let ns = namespace(&r, "M");
    assert_eq!(ns.qualified_name, "M");
    assert_child_qualified(&r, "g", "M");
}

#[test]
fn plain_dotted_module_unchanged() {
    let r = extract("module A.B\n\nlet h z = z\n");
    let ns = namespace(&r, "A.B");
    assert_eq!(ns.qualified_name, "A.B");
    assert_child_qualified(&r, "h", "A.B");
}

#[test]
fn method_defn_emits_method_symbol_parented_to_type() {
    let r = extract(concat!(
        "module MyModule\n",
        "type Helper() =\n",
        "    member this.LibCall x = x + 1\n",
        "    member this.Value = 42\n",
        "let foo x = x\n",
    ));
    let type_idx = r
        .symbols
        .iter()
        .position(|s| s.name == "Helper" && s.kind == SymbolKind::Class)
        .unwrap_or_else(|| {
            panic!(
                "expected Class 'Helper'; got {:?}",
                r.symbols
                    .iter()
                    .map(|s| (&s.name, s.kind))
                    .collect::<Vec<_>>()
            )
        });
    let method = r
        .symbols
        .iter()
        .find(|s| s.name == "LibCall")
        .unwrap_or_else(|| {
            panic!(
                "expected member symbol 'LibCall'; got {:?}",
                r.symbols
                    .iter()
                    .map(|s| (&s.name, s.kind))
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        method.kind,
        SymbolKind::Method,
        "a member with argument patterns is a Method"
    );
    assert_eq!(
        method.parent_index,
        Some(type_idx),
        "member parents onto the enclosing type"
    );
    assert_eq!(method.qualified_name, "MyModule.Helper.LibCall");
    assert_eq!(method.scope_path.as_deref(), Some("MyModule.Helper"));
}

#[test]
fn property_defn_emits_property_symbol_parented_to_type() {
    let r = extract(concat!(
        "module MyModule\n",
        "type Helper() =\n",
        "    member this.LibCall x = x + 1\n",
        "    member this.Value = 42\n",
        "let foo x = x\n",
    ));
    let type_idx = r
        .symbols
        .iter()
        .position(|s| s.name == "Helper" && s.kind == SymbolKind::Class)
        .expect("Class Helper");
    let prop = r
        .symbols
        .iter()
        .find(|s| s.name == "Value")
        .unwrap_or_else(|| {
            panic!(
                "expected member symbol 'Value'; got {:?}",
                r.symbols
                    .iter()
                    .map(|s| (&s.name, s.kind))
                    .collect::<Vec<_>>()
            )
        });
    assert_eq!(
        prop.kind,
        SymbolKind::Property,
        "a member without argument patterns is a Property"
    );
    assert_eq!(prop.parent_index, Some(type_idx));
}
