// Tests for property_decl.rs — the type evidence a PHP property declaration carries.

use crate::languages::php::extract;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol};

struct Extraction {
    symbols: Vec<ExtractedSymbol>,
    refs: Vec<ExtractedRef>,
}

/// Extract a class body holding a single declaration under test.
fn extract_class(body: &str) -> Extraction {
    let result = extract::extract(&format!("<?php\nnamespace Fx;\nclass Host\n{{\n{body}\n}}\n"));
    Extraction {
        symbols: result.symbols,
        refs: result.refs,
    }
}

/// TypeRef target names whose source symbol is the property named `prop`.
fn property_type_refs(extraction: &Extraction, prop: &str) -> Vec<String> {
    let idx = extraction
        .symbols
        .iter()
        .position(|s| s.name == prop)
        .unwrap_or_else(|| panic!("no property `{prop}` in {:?}", extraction.symbols));
    extraction
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::TypeRef && r.source_symbol_index == idx)
        .map(|r| r.target_name.clone())
        .collect()
}

fn doc_comment(extraction: &Extraction, prop: &str) -> Option<String> {
    extraction
        .symbols
        .iter()
        .find(|s| s.name == prop)
        .and_then(|s| s.doc_comment.clone())
}

#[test]
fn a_qualified_var_tag_types_an_unhinted_property() {
    let extraction = extract_class("    /** @var \\Fx\\Factory */\n    protected $factory;");
    assert_eq!(
        property_type_refs(&extraction, "factory"),
        vec!["\\Fx\\Factory".to_string()]
    );
}

#[test]
fn a_native_hint_suppresses_the_doc_tag() {
    let extraction = extract_class("    /** @var \\Fx\\Other */\n    protected Factory $f;");
    assert_eq!(
        property_type_refs(&extraction, "f"),
        vec!["Factory".to_string()]
    );
}

#[test]
fn a_nullable_var_tag_collapses_to_its_class() {
    for tag in ["?\\Fx\\Factory", "\\Fx\\Factory|null", "null|\\Fx\\Factory"] {
        let extraction = extract_class(&format!("    /** @var {tag} */\n    protected $factory;"));
        assert_eq!(
            property_type_refs(&extraction, "factory"),
            vec!["\\Fx\\Factory".to_string()],
            "tag `{tag}`"
        );
    }
}

#[test]
fn an_ambiguous_var_tag_abstains() {
    for tag in [
        "\\Fx\\A|\\Fx\\B",
        "\\Fx\\Factory[]",
        "array<int, \\Fx\\Factory>",
        "array{a: int}",
    ] {
        let extraction = extract_class(&format!("    /** @var {tag} */\n    protected $many;"));
        assert!(
            property_type_refs(&extraction, "many").is_empty(),
            "tag `{tag}` must abstain, got {:?}",
            property_type_refs(&extraction, "many")
        );
    }
}

#[test]
fn a_scalar_var_tag_emits_no_reference() {
    for tag in ["string", "mixed"] {
        let extraction = extract_class(&format!("    /** @var {tag} */\n    protected $value;"));
        assert!(
            property_type_refs(&extraction, "value").is_empty(),
            "tag `{tag}` must emit no TypeRef"
        );
    }
}

#[test]
fn the_property_records_its_doc_block() {
    let extraction = extract_class(
        "    /**\n     * The HTTP factory.\n     *\n     * @var \\Fx\\Factory\n     */\n    protected $factory;",
    );
    let doc = doc_comment(&extraction, "factory").expect("property records its docblock");
    assert!(doc.contains("@var"), "doc block was {doc:?}");
    assert_eq!(
        property_type_refs(&extraction, "factory"),
        vec!["\\Fx\\Factory".to_string()]
    );
}

#[test]
fn a_block_separated_from_the_property_is_not_read() {
    let extraction = extract_class(
        "    /** @var \\Fx\\Factory */\n    // a plain comment breaks the association\n    protected $factory;",
    );
    assert_eq!(doc_comment(&extraction, "factory"), None);
    assert!(property_type_refs(&extraction, "factory").is_empty());
}

#[test]
fn two_properties_in_one_declaration_share_the_tag() {
    let extraction = extract_class("    /** @var \\Fx\\Factory */\n    protected $a, $b;");
    assert_eq!(
        property_type_refs(&extraction, "a"),
        vec!["\\Fx\\Factory".to_string()]
    );
    assert_eq!(
        property_type_refs(&extraction, "b"),
        vec!["\\Fx\\Factory".to_string()]
    );
}
