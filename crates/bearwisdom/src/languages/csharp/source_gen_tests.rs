// =============================================================================
// csharp/source_gen_tests.rs — unit tests for record member synthesis
// =============================================================================

use super::source_gen::_test_synthesize;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Synthesized qualified names, sorted.
fn qnames(source: &str) -> Vec<String> {
    let mut v: Vec<String> = _test_synthesize(source)
        .symbols
        .into_iter()
        .map(|s| s.qualified_name)
        .collect();
    v.sort();
    v
}

/// The signature of the synthesized symbol with qualified name `qn`.
fn signature_for(source: &str, qn: &str) -> Option<String> {
    _test_synthesize(source)
        .symbols
        .into_iter()
        .find(|s| s.qualified_name == qn)
        .and_then(|s| s.signature)
}

// ---------------------------------------------------------------------------
// Discriminator: only records synthesize a Deconstruct
// ---------------------------------------------------------------------------

#[test]
fn plain_positional_class_yields_no_deconstruct() {
    // A primary-constructor *class* (C# 12) is also `class Point(int X, int Y)`
    // syntax but is NOT a record — it has no compiler-synthesized Deconstruct.
    // The source-text `record`-keyword discriminator must reject it.
    let q = qnames("namespace App { public class Point(int X, int Y); }");
    assert!(
        !q.iter().any(|n| n.ends_with(".Deconstruct")),
        "a non-record positional class must not synthesize Deconstruct; got {q:?}"
    );
}

#[test]
fn plain_class_with_record_in_name_yields_nothing() {
    // `RecordStore` contains "record" as a substring — must not match.
    let q = qnames("namespace App { public class RecordStore(int Id); }");
    assert!(
        !q.iter().any(|n| n.ends_with(".Deconstruct")),
        "a class whose name contains 'record' as a substring must not synthesize; got {q:?}"
    );
}

// ---------------------------------------------------------------------------
// Positional record → Deconstruct
// ---------------------------------------------------------------------------

#[test]
fn positional_record_synthesizes_deconstruct() {
    let q = qnames("namespace App { public record Point(int X, int Y); }");
    assert!(
        q.contains(&"App.Point.Deconstruct".to_string()),
        "a positional record must synthesize Deconstruct; got {q:?}"
    );
}

#[test]
fn deconstruct_signature_carries_out_params_with_types() {
    let sig = signature_for("namespace App { public record Point(int X, int Y); }", "App.Point.Deconstruct")
        .expect("Deconstruct must be synthesized");
    assert!(sig.contains("out int X"), "Deconstruct sig must carry `out int X`; got {sig}");
    assert!(sig.contains("out int Y"), "Deconstruct sig must carry `out int Y`; got {sig}");
}

#[test]
fn deconstruct_is_method_kind_void_return() {
    let s = _test_synthesize("namespace App { public record Point(int X, int Y); }");
    let dc = s.symbols.iter().find(|s| s.name == "Deconstruct").expect("Deconstruct synthesized");
    assert_eq!(dc.kind, SymbolKind::Method, "Deconstruct must be a Method");
    // Deconstruct returns void — no return-type ref points at it.
    let idx = s.symbols.iter().position(|sy| sy.qualified_name == "App.Point.Deconstruct").unwrap();
    assert!(
        !s.refs.iter().any(|rf| rf.source_symbol_index == idx && rf.kind == EdgeKind::TypeRef),
        "void Deconstruct must emit no return-type ref"
    );
}

#[test]
fn non_positional_record_synthesizes_nothing() {
    // `record Person { ... }` with no positional parameter list → no positional
    // properties → no Deconstruct.
    let q = qnames("namespace App { public record Person { public string Name { get; init; } } }");
    assert!(
        !q.iter().any(|n| n.ends_with(".Deconstruct")),
        "a non-positional record must not synthesize Deconstruct; got {q:?}"
    );
}

#[test]
fn positional_record_with_body_excludes_body_property() {
    // A positional record may also carry a body property. Deconstruct's params
    // are the POSITIONAL ones only — the body `Extra` must not appear.
    let src = "namespace App {\n  public record Point(int X, int Y) {\n    public string Extra { get; init; }\n  }\n}";
    let sig = signature_for(src, "App.Point.Deconstruct").expect("Deconstruct synthesized");
    assert!(sig.contains("out int X") && sig.contains("out int Y"), "positional params must be present; got {sig}");
    assert!(!sig.contains("Extra"), "body property must not appear in Deconstruct; got {sig}");
}

// ---------------------------------------------------------------------------
// Dedup: hand-written Deconstruct wins
// ---------------------------------------------------------------------------

#[test]
fn hand_written_deconstruct_not_duplicated() {
    let src = "namespace App { public record Point(int X, int Y) {\n    public void Deconstruct(out int x, out int y) { x = X; y = Y; }\n} }";
    let q = qnames(src);
    let dcs: Vec<_> = q.iter().filter(|n| n.ends_with(".Deconstruct")).collect();
    assert_eq!(dcs.len(), 0, "hand-written Deconstruct must suppress synthesis; got {q:?}");
}

// ---------------------------------------------------------------------------
// Chain-through-property: the extractor surface the chain rides + Deconstruct
// ---------------------------------------------------------------------------

#[test]
fn positional_property_exists_with_typed_signature() {
    // The chain `dto.Category.Id` rides the positional property's signature,
    // which the extractor already emits — the recognizer must not regress it.
    let src = "namespace App { public record UserDto(string Name, Category Category); public class Category { public int Id; } }";
    let r = super::extract::extract(src);
    let prop = r
        .symbols
        .iter()
        .find(|s| s.qualified_name == "App.UserDto.Category")
        .expect("positional property App.UserDto.Category must be extracted");
    assert_eq!(prop.kind, SymbolKind::Property);
    let sig = prop.signature.as_deref().unwrap_or("");
    assert!(sig.contains("Category"), "property signature must carry its `Category` type; got {sig}");
}

#[test]
fn deconstruct_carries_complex_property_type() {
    let src = "namespace App { public record UserDto(string Name, Category Category); }";
    let sig = signature_for(src, "App.UserDto.Deconstruct").expect("Deconstruct synthesized");
    assert!(sig.contains("out Category Category"), "Deconstruct must carry `out Category Category`; got {sig}");
}

// ---------------------------------------------------------------------------
// End-to-end: Deconstruct resolves through the index after the splice
// ---------------------------------------------------------------------------

#[test]
fn deconstruct_resolves_through_index() {
    use crate::indexer::resolve::engine::{SymbolIndex, SymbolLookup};
    use crate::types::{FlowMeta, ParsedFile};
    use std::collections::HashMap;

    let source = "namespace App { public record Point(int X, int Y); }";
    let r = super::extract::extract(source);
    let synth = _test_synthesize(source);

    // Merge synthesized symbols/refs onto the extractor output (mirrors parse_file splice).
    let base = r.symbols.len();
    let mut all_symbols = r.symbols.clone();
    all_symbols.extend(synth.symbols.clone());
    let mut all_refs = r.refs.clone();
    for mut sref in synth.refs.clone() {
        sref.source_symbol_index += base;
        all_refs.push(sref);
    }

    let pf = ParsedFile {
        path: "src/Point.cs".to_string(),
        language: "csharp".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 1,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols: all_symbols.clone(),
        refs: all_refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    };

    let mut id_map: HashMap<(String, String), i64> = HashMap::new();
    for (i, sym) in all_symbols.iter().enumerate() {
        id_map.insert(("src/Point.cs".to_string(), sym.qualified_name.clone()), i as i64 + 1);
    }

    let index = SymbolIndex::build(&[pf], &id_map);

    // Deconstruct must be a reachable member of Point after the splice — this is
    // the headline `point.Deconstruct(out _, out _)` / `var (a, b) = point` proof.
    assert!(
        index.by_qualified_name("App.Point.Deconstruct").is_some(),
        "App.Point.Deconstruct must be in the index after merging synthesized symbols"
    );
    let members = index.members_of("App.Point");
    assert!(
        members.iter().any(|si| si.name == "Deconstruct"),
        "Deconstruct must be a member of App.Point so positional deconstruction binds"
    );
}
