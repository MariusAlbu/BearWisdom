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

/// The return-type `TypeRef` target carried by the synthesized symbol `qn`, if
/// any. The synthesized ref's `source_symbol_index` is relative to the
/// synthesized symbol list.
fn return_ref_for(source: &str, qn: &str) -> Option<String> {
    let synth = _test_synthesize(source);
    let idx = synth.symbols.iter().position(|s| s.qualified_name == qn)?;
    synth
        .refs
        .iter()
        .find(|r| r.source_symbol_index == idx && r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.clone())
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
// Inheriting positional records: Deconstruct includes the base's positional
// members first, then the derived ones (the real C# rule). The base reference
// of `record Derived(...) : Base(...)` is a `primary_constructor_base_type`
// node carrying an `argument_list`, so the extractor must emit its `Inherits`
// edge for the recognizer to find the base.
// ---------------------------------------------------------------------------

#[test]
fn derived_record_base_inherits_edge_emitted() {
    // The base-with-arguments form `: Base(X)` parses as a
    // `primary_constructor_base_type`, not a bare name — the extractor must
    // still emit the `Inherits` edge to `Base`.
    let src = "namespace App { public record Base(int X); public record Derived(int X, int Z) : Base(X); }";
    let r = super::extract::extract(src);
    let derived_idx = r
        .symbols
        .iter()
        .position(|s| s.qualified_name == "App.Derived")
        .expect("derived record extracted");
    assert!(
        r.refs.iter().any(|rf| rf.source_symbol_index == derived_idx
            && rf.kind == EdgeKind::Inherits
            && rf.target_name == "Base"),
        "a derived record `: Base(X)` must emit an Inherits edge to Base; got {:?}",
        r.refs.iter().filter(|rf| rf.source_symbol_index == derived_idx).map(|rf| (&rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn derived_record_deconstruct_includes_base_positional_params_first() {
    // `record Derived(int X, int Z) : Base(X)` where `record Base(int X)` makes
    // the compiler generate `Deconstruct(out int X, out int Z)` — base members
    // FIRST, then the derived-only ones. (`X` is the base param threaded through
    // the base ctor; the derived positional list here is `(int X, int Z)`, but
    // the synthesized Deconstruct must reflect base-then-derived ordering and
    // not double the shared `X`.)
    let src = "namespace App { public record Base(int X); public record Derived(int Z) : Base(0); }";
    let sig = signature_for(src, "App.Derived.Deconstruct").expect("derived Deconstruct synthesized");
    let x_at = sig.find("out int X").expect("base param X must be present");
    let z_at = sig.find("out int Z").expect("derived param Z must be present");
    assert!(
        x_at < z_at,
        "base positional member X must precede derived Z in Deconstruct; got {sig}"
    );
}

#[test]
fn derived_record_with_external_base_uses_derived_params_only() {
    // When the base does not resolve to an in-project positional record (here it
    // is absent / external), the Deconstruct soundly carries only the derived
    // positional members — no over-reach, no manufactured base params.
    let src = "namespace App { public record Derived(int Z) : SomeExternalBase(0); }";
    let sig = signature_for(src, "App.Derived.Deconstruct").expect("derived Deconstruct synthesized");
    assert!(sig.contains("out int Z"), "derived param Z must be present; got {sig}");
    assert!(
        !sig.contains("SomeExternalBase"),
        "an unresolved base must not contribute params; got {sig}"
    );
}

#[test]
fn derived_record_skips_non_record_base() {
    // A derived record whose base is a plain (non-positional) class contributes
    // no base params — only a base that is itself a positional record does.
    let src = "namespace App { public class Plain { } public record Derived(int Z) : Plain; }";
    let sig = signature_for(src, "App.Derived.Deconstruct").expect("derived Deconstruct synthesized");
    assert_eq!(
        sig, "void Deconstruct(out int Z)",
        "a non-record base must contribute no positional params; got {sig}"
    );
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

// ---------------------------------------------------------------------------
// MVVM Community Toolkit [ObservableProperty] — field → public property
// ---------------------------------------------------------------------------

#[test]
fn observable_property_synthesizes_public_property() {
    // `[ObservableProperty] private string _firstName;` makes the generator emit
    // `public string FirstName { get; set; }`. The field→property name transform
    // strips one leading underscore and uppercases the first remaining letter.
    let q = qnames("namespace App { public partial class VM { [ObservableProperty] private string _firstName; } }");
    assert!(
        q.contains(&"App.VM.FirstName".to_string()),
        "[ObservableProperty] field `_firstName` must synthesize property `FirstName`; got {q:?}"
    );
    let s = _test_synthesize("namespace App { public partial class VM { [ObservableProperty] private string _firstName; } }");
    let prop = s.symbols.iter().find(|s| s.qualified_name == "App.VM.FirstName").expect("FirstName synthesized");
    assert_eq!(prop.kind, SymbolKind::Property, "generated FirstName must be a Property");
}

#[test]
fn observable_property_no_underscore_capitalizes() {
    // A field with no leading underscore (`name`) → property `Name`.
    let q = qnames("namespace App { public partial class VM { [ObservableProperty] private string name; } }");
    assert!(
        q.contains(&"App.VM.Name".to_string()),
        "[ObservableProperty] field `name` must synthesize property `Name`; got {q:?}"
    );
}

#[test]
fn observable_property_carries_field_type_return_ref() {
    // A primitive field type emits NO return-type ref (Lombok/data-class convention).
    assert_eq!(
        return_ref_for(
            "namespace App { public partial class VM { [ObservableProperty] private string _firstName; } }",
            "App.VM.FirstName",
        ),
        None,
        "a primitive `string` property carries no return-type ref"
    );
    // A complex field type emits a return-type ref so `vm.Current.X()` types through.
    assert_eq!(
        return_ref_for(
            "namespace App { public partial class VM { [ObservableProperty] private User _current; } }",
            "App.VM.Current",
        ),
        Some("User".to_string()),
        "a complex `User` property must carry a return-type ref `User`"
    );
}

#[test]
fn observable_property_synthesizes_on_changed_hooks() {
    // The generator emits `partial void On{Name}Changing(...)` and
    // `On{Name}Changed(...)` hooks — a call to them must resolve to a member.
    let q = qnames("namespace App { public partial class VM { [ObservableProperty] private string _firstName; } }");
    assert!(
        q.contains(&"App.VM.OnFirstNameChanging".to_string()),
        "[ObservableProperty] must synthesize On{{Name}}Changing hook; got {q:?}"
    );
    assert!(
        q.contains(&"App.VM.OnFirstNameChanged".to_string()),
        "[ObservableProperty] must synthesize On{{Name}}Changed hook; got {q:?}"
    );
}

#[test]
fn plain_field_without_attribute_synthesizes_nothing() {
    // No attribute → no synthesis.
    let q = qnames("namespace App { public partial class VM { private string _firstName; } }");
    assert!(
        !q.iter().any(|n| n == "App.VM.FirstName"),
        "a plain field with no [ObservableProperty] must synthesize nothing; got {q:?}"
    );
}

#[test]
fn plain_field_below_inline_attributed_field_does_not_leak() {
    // An inline-attributed field `[ObservableProperty] private string _firstName;`
    // followed directly (no blank line) by a plain `private string _lastName;`
    // must NOT leak the attribute downward. The upward scan from `_lastName`
    // lands on the inline-decl line above, which is `[...]`-shaped at its start
    // but is a full member declaration, not a standalone attribute line.
    let src = "namespace App { public partial class VM {\n  [ObservableProperty] private string _firstName;\n  private string _lastName;\n} }";
    let q = qnames(src);
    assert!(
        q.contains(&"App.VM.FirstName".to_string()),
        "the inline-attributed field must still synthesize FirstName; got {q:?}"
    );
    assert!(
        !q.iter().any(|n| n == "App.VM.LastName"),
        "the plain field below must NOT synthesize LastName — no attribute applies to it; got {q:?}"
    );
}

#[test]
fn field_named_observableproperty_substring_rejected() {
    // A field whose declared TYPE merely contains the attribute text as a
    // substring must not be treated as attributed — whole-token match only.
    let q = qnames("namespace App { public partial class VM { private ObservablePropertyHolder _holder; } }");
    assert!(
        q.is_empty() || !q.iter().any(|n| n.starts_with("App.VM.Holder")),
        "a field whose type contains `ObservableProperty` as a substring must not synthesize; got {q:?}"
    );
}

#[test]
fn hand_written_property_wins() {
    // A class that already declares `public string FirstName { get; set; }` plus
    // `[ObservableProperty] private string _firstName;` must not double up.
    let src = "namespace App { public partial class VM {\n  public string FirstName { get; set; }\n  [ObservableProperty] private string _firstName;\n} }";
    let count = _test_synthesize(src)
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "App.VM.FirstName")
        .count();
    assert_eq!(count, 0, "hand-written FirstName must suppress the synthesized property");
}

// ---------------------------------------------------------------------------
// MVVM Community Toolkit [RelayCommand] — method → command property
// ---------------------------------------------------------------------------

#[test]
fn relay_command_synthesizes_command_property() {
    // `[RelayCommand] private void Save() {}` → `public IRelayCommand SaveCommand { get; }`.
    let src = "namespace App { public partial class VM { [RelayCommand] private void Save() { } } }";
    let q = qnames(src);
    assert!(
        q.contains(&"App.VM.SaveCommand".to_string()),
        "[RelayCommand] method `Save` must synthesize `SaveCommand`; got {q:?}"
    );
    let sig = signature_for(src, "App.VM.SaveCommand").expect("SaveCommand synthesized");
    assert!(sig.contains("IRelayCommand"), "SaveCommand sig must carry `IRelayCommand`; got {sig}");
    assert_eq!(
        return_ref_for(src, "App.VM.SaveCommand"),
        Some("IRelayCommand".to_string()),
        "SaveCommand must carry a return-type ref to IRelayCommand"
    );
    let s = _test_synthesize(src);
    let prop = s.symbols.iter().find(|s| s.qualified_name == "App.VM.SaveCommand").unwrap();
    assert_eq!(prop.kind, SymbolKind::Property, "SaveCommand must be a Property");
}

#[test]
fn relay_command_async_synthesizes_async_command() {
    // `async Task SaveAsync()` → `IAsyncRelayCommand SaveAsyncCommand`.
    let src = "namespace App { public partial class VM { [RelayCommand] private async System.Threading.Tasks.Task SaveAsync() { } } }";
    let q = qnames(src);
    assert!(
        q.contains(&"App.VM.SaveAsyncCommand".to_string()),
        "[RelayCommand] async method must synthesize `{{Name}}Command`; got {q:?}"
    );
    let sig = signature_for(src, "App.VM.SaveAsyncCommand").expect("SaveAsyncCommand synthesized");
    assert!(sig.contains("IAsyncRelayCommand"), "async command sig must carry `IAsyncRelayCommand`; got {sig}");
    assert_eq!(
        return_ref_for(src, "App.VM.SaveAsyncCommand"),
        Some("IAsyncRelayCommand".to_string()),
        "async command must carry a return-type ref to IAsyncRelayCommand"
    );
}

#[test]
fn plain_method_without_relay_attribute_synthesizes_nothing() {
    let q = qnames("namespace App { public partial class VM { private void Save() { } } }");
    assert!(
        !q.iter().any(|n| n == "App.VM.SaveCommand"),
        "a plain method without [RelayCommand] must synthesize no command; got {q:?}"
    );
}

#[test]
fn hand_written_command_property_wins() {
    let src = "namespace App { public partial class VM {\n  public IRelayCommand SaveCommand { get; }\n  [RelayCommand] private void Save() { }\n} }";
    let count = _test_synthesize(src)
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "App.VM.SaveCommand")
        .count();
    assert_eq!(count, 0, "hand-written SaveCommand must suppress the synthesized command property");
}

// ---------------------------------------------------------------------------
// End-to-end: an [ObservableProperty] resolves through the index
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// MVVM Community Toolkit [ObservableObject] / ObservableObject base → the
// change-notification surface (SetProperty / OnPropertyChanged / OnPropertyChanging)
// ---------------------------------------------------------------------------

#[test]
fn observable_object_attribute_synthesizes_change_notification_surface() {
    // `[ObservableObject] public partial class VM { }` makes the generator inject
    // the INotifyPropertyChanged helper surface every property setter calls.
    let src = "namespace App { [ObservableObject] public partial class VM { } }";
    let q = qnames(src);
    assert!(
        q.contains(&"App.VM.SetProperty".to_string()),
        "[ObservableObject] must synthesize SetProperty; got {q:?}"
    );
    assert!(
        q.contains(&"App.VM.OnPropertyChanged".to_string()),
        "[ObservableObject] must synthesize OnPropertyChanged; got {q:?}"
    );
    assert!(
        q.contains(&"App.VM.OnPropertyChanging".to_string()),
        "[ObservableObject] must synthesize OnPropertyChanging; got {q:?}"
    );
}

#[test]
fn observable_object_base_class_synthesizes_change_notification_surface() {
    // The common form is inheritance, not the attribute: `class VM : ObservableObject`.
    let src = "namespace App { public partial class VM : ObservableObject { } }";
    let q = qnames(src);
    assert!(
        q.contains(&"App.VM.SetProperty".to_string()),
        "a class inheriting ObservableObject must synthesize SetProperty; got {q:?}"
    );
    assert!(
        q.contains(&"App.VM.OnPropertyChanged".to_string()),
        "a class inheriting ObservableObject must synthesize OnPropertyChanged; got {q:?}"
    );
}

#[test]
fn observable_validator_base_class_synthesizes_set_property() {
    // ObservableValidator/ObservableRecipient also carry the SetProperty surface.
    let src = "namespace App { public partial class VM : ObservableValidator { } }";
    let q = qnames(src);
    assert!(
        q.contains(&"App.VM.SetProperty".to_string()),
        "a class inheriting ObservableValidator must synthesize SetProperty; got {q:?}"
    );
}

#[test]
fn inotify_property_changed_attribute_synthesizes_surface() {
    let src = "namespace App { [INotifyPropertyChanged] public partial class VM { } }";
    let q = qnames(src);
    assert!(
        q.contains(&"App.VM.OnPropertyChanged".to_string()),
        "[INotifyPropertyChanged] must synthesize OnPropertyChanged; got {q:?}"
    );
}

#[test]
fn set_property_signature_and_kind() {
    let src = "namespace App { [ObservableObject] public partial class VM { } }";
    let s = _test_synthesize(src);
    let sp = s.symbols.iter().find(|s| s.qualified_name == "App.VM.SetProperty").expect("SetProperty synthesized");
    assert_eq!(sp.kind, SymbolKind::Method, "SetProperty must be a Method");
    let sig = sp.signature.as_deref().unwrap_or("");
    assert!(sig.starts_with("bool SetProperty"), "SetProperty must return bool; got {sig}");
    // bool return → no return-type ref (the void/primitive convention).
    let idx = s.symbols.iter().position(|sy| sy.qualified_name == "App.VM.SetProperty").unwrap();
    assert!(
        !s.refs.iter().any(|r| r.source_symbol_index == idx && r.kind == EdgeKind::TypeRef),
        "bool-returning SetProperty must emit no return-type ref"
    );
}

#[test]
fn plain_class_synthesizes_no_change_notification_surface() {
    // A class with neither the attribute nor an MVVM base must synthesize nothing.
    let src = "namespace App { public class VM { } }";
    let q = qnames(src);
    assert!(
        !q.iter().any(|n| n == "App.VM.SetProperty" || n == "App.VM.OnPropertyChanged"),
        "a plain class must not synthesize the change-notification surface; got {q:?}"
    );
}

#[test]
fn unrelated_base_class_synthesizes_nothing() {
    // A class inheriting an unrelated base (not an MVVM observable base) must not
    // gain SetProperty — the base-name discriminator is a whole-name match.
    let src = "namespace App { public class VM : SomeOtherBase { } }";
    let q = qnames(src);
    assert!(
        !q.iter().any(|n| n == "App.VM.SetProperty"),
        "an unrelated base must not synthesize SetProperty; got {q:?}"
    );
}

#[test]
fn hand_written_set_property_wins() {
    // A class already declaring SetProperty must not double up.
    let src = "namespace App { [ObservableObject] public partial class VM {\n  protected bool SetProperty<T>(ref T f, T v) => true;\n} }";
    let count = _test_synthesize(src)
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "App.VM.SetProperty")
        .count();
    assert_eq!(count, 0, "hand-written SetProperty must suppress synthesis");
}

// ---------------------------------------------------------------------------
// ObservableValidator → validation surface (in addition to the common members)
// ---------------------------------------------------------------------------

#[test]
fn observable_validator_synthesizes_validation_surface() {
    // `class VM : ObservableValidator` injects the data-validation surface on top
    // of the common change-notification members: HasErrors, ValidateProperty,
    // ValidateAllProperties, ClearAllErrors, GetErrors, TrySetProperty.
    let src = "namespace App { public partial class VM : ObservableValidator { } }";
    let q = qnames(src);
    for member in [
        "App.VM.HasErrors",
        "App.VM.ValidateProperty",
        "App.VM.ValidateAllProperties",
        "App.VM.ClearAllErrors",
        "App.VM.GetErrors",
        "App.VM.TrySetProperty",
    ] {
        assert!(
            q.contains(&member.to_string()),
            "ObservableValidator must synthesize {member}; got {q:?}"
        );
    }
    // It still carries the common change-notification surface too.
    assert!(
        q.contains(&"App.VM.SetProperty".to_string()),
        "ObservableValidator must also carry the common SetProperty; got {q:?}"
    );
}

#[test]
fn observable_object_does_not_synthesize_validation_surface() {
    // A plain ObservableObject host must NOT gain the validator-only members —
    // the validation surface is gated on the ObservableValidator base specifically.
    let src = "namespace App { public partial class VM : ObservableObject { } }";
    let q = qnames(src);
    assert!(
        !q.iter().any(|n| n == "App.VM.ValidateAllProperties" || n == "App.VM.HasErrors"),
        "an ObservableObject host must not gain the validator surface; got {q:?}"
    );
}

#[test]
fn has_errors_is_bool_and_carries_no_return_ref() {
    let src = "namespace App { public partial class VM : ObservableValidator { } }";
    let s = _test_synthesize(src);
    let he = s.symbols.iter().find(|s| s.qualified_name == "App.VM.HasErrors").expect("HasErrors synthesized");
    let sig = he.signature.as_deref().unwrap_or("");
    assert!(sig.starts_with("bool HasErrors"), "HasErrors must return bool; got {sig}");
    assert_eq!(
        return_ref_for(src, "App.VM.HasErrors"),
        None,
        "bool HasErrors must carry no return-type ref"
    );
}

// ---------------------------------------------------------------------------
// ObservableRecipient → messaging surface (in addition to the common members)
// ---------------------------------------------------------------------------

#[test]
fn observable_recipient_synthesizes_messaging_surface() {
    // `class VM : ObservableRecipient` injects the messaging surface: Messenger,
    // IsActive, Broadcast, OnActivated, OnDeactivated — on top of the common
    // change-notification members.
    let src = "namespace App { public partial class VM : ObservableRecipient { } }";
    let q = qnames(src);
    for member in [
        "App.VM.Messenger",
        "App.VM.IsActive",
        "App.VM.Broadcast",
        "App.VM.OnActivated",
        "App.VM.OnDeactivated",
    ] {
        assert!(
            q.contains(&member.to_string()),
            "ObservableRecipient must synthesize {member}; got {q:?}"
        );
    }
    assert!(
        q.contains(&"App.VM.SetProperty".to_string()),
        "ObservableRecipient must also carry the common SetProperty; got {q:?}"
    );
}

#[test]
fn messenger_property_carries_imessenger_return_ref() {
    // `Messenger` is a property of interface type IMessenger — a complex
    // (non-primitive) return type, so it carries a return-type ref so
    // `vm.Messenger.Send(...)` types through to the (hydrated) interface.
    let src = "namespace App { public partial class VM : ObservableRecipient { } }";
    assert_eq!(
        return_ref_for(src, "App.VM.Messenger"),
        Some("IMessenger".to_string()),
        "Messenger must carry a return-type ref to IMessenger"
    );
    let s = _test_synthesize(src);
    let m = s.symbols.iter().find(|s| s.qualified_name == "App.VM.Messenger").expect("Messenger synthesized");
    assert_eq!(m.kind, SymbolKind::Property, "Messenger must be a Property");
}

#[test]
fn observable_object_does_not_synthesize_messaging_surface() {
    let src = "namespace App { public partial class VM : ObservableObject { } }";
    let q = qnames(src);
    assert!(
        !q.iter().any(|n| n == "App.VM.Messenger" || n == "App.VM.Broadcast"),
        "an ObservableObject host must not gain the messaging surface; got {q:?}"
    );
}

#[test]
fn hand_written_validator_member_wins() {
    // A class already declaring ValidateAllProperties must not double up.
    let src = "namespace App { public partial class VM : ObservableValidator {\n  public void ValidateAllProperties() { }\n} }";
    let count = _test_synthesize(src)
        .symbols
        .iter()
        .filter(|s| s.qualified_name == "App.VM.ValidateAllProperties")
        .count();
    assert_eq!(count, 0, "hand-written ValidateAllProperties must suppress synthesis");
}

#[test]
fn observable_recipient_resolves_messenger_through_index() {
    use crate::indexer::resolve::engine::{SymbolIndex, SymbolLookup};
    use crate::types::{FlowMeta, ParsedFile};
    use std::collections::HashMap;

    let source = "namespace App { public partial class VM : ObservableRecipient { } }";
    let r = super::extract::extract(source);
    let synth = _test_synthesize(source);

    let base = r.symbols.len();
    let mut all_symbols = r.symbols.clone();
    all_symbols.extend(synth.symbols.clone());
    let mut all_refs = r.refs.clone();
    for mut sref in synth.refs.clone() {
        sref.source_symbol_index += base;
        all_refs.push(sref);
    }

    let pf = ParsedFile {
        path: "src/VM.cs".to_string(),
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
        id_map.insert(("src/VM.cs".to_string(), sym.qualified_name.clone()), i as i64 + 1);
    }

    let index = SymbolIndex::build(&[pf], &id_map);

    let members = index.members_of("App.VM");
    assert!(
        members.iter().any(|si| si.name == "Messenger"),
        "Messenger must be a member of App.VM so `vm.Messenger` binds"
    );
    assert!(
        members.iter().any(|si| si.name == "Broadcast"),
        "Broadcast must be a member of App.VM so `vm.Broadcast(...)` binds"
    );
}

#[test]
fn observable_object_resolves_set_property_through_index() {
    use crate::indexer::resolve::engine::{SymbolIndex, SymbolLookup};
    use crate::types::{FlowMeta, ParsedFile};
    use std::collections::HashMap;

    let source = "namespace App { [ObservableObject] public partial class VM { } }";
    let r = super::extract::extract(source);
    let synth = _test_synthesize(source);

    let base = r.symbols.len();
    let mut all_symbols = r.symbols.clone();
    all_symbols.extend(synth.symbols.clone());
    let mut all_refs = r.refs.clone();
    for mut sref in synth.refs.clone() {
        sref.source_symbol_index += base;
        all_refs.push(sref);
    }

    let pf = ParsedFile {
        path: "src/VM.cs".to_string(),
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
        id_map.insert(("src/VM.cs".to_string(), sym.qualified_name.clone()), i as i64 + 1);
    }

    let index = SymbolIndex::build(&[pf], &id_map);

    let members = index.members_of("App.VM");
    assert!(
        members.iter().any(|si| si.name == "SetProperty"),
        "SetProperty must be a member of App.VM so a `SetProperty(ref _x, value)` call binds"
    );
}

#[test]
fn observable_property_resolves_through_index() {
    use crate::indexer::resolve::engine::{SymbolIndex, SymbolLookup};
    use crate::types::{FlowMeta, ParsedFile};
    use std::collections::HashMap;

    let source = "namespace App { public partial class VM { [ObservableProperty] private User _current; } public class User { public int Id; } }";
    let r = super::extract::extract(source);
    let synth = _test_synthesize(source);

    let base = r.symbols.len();
    let mut all_symbols = r.symbols.clone();
    all_symbols.extend(synth.symbols.clone());
    let mut all_refs = r.refs.clone();
    for mut sref in synth.refs.clone() {
        sref.source_symbol_index += base;
        all_refs.push(sref);
    }

    let pf = ParsedFile {
        path: "src/VM.cs".to_string(),
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
        id_map.insert(("src/VM.cs".to_string(), sym.qualified_name.clone()), i as i64 + 1);
    }

    let index = SymbolIndex::build(&[pf], &id_map);

    assert!(
        index.by_qualified_name("App.VM.Current").is_some(),
        "App.VM.Current must be in the index after merging synthesized symbols"
    );
    let members = index.members_of("App.VM");
    assert!(
        members.iter().any(|si| si.name == "Current"),
        "Current must be a member of App.VM so `vm.Current` binds"
    );
    // A Property's declared type lands in the field-type map (the surface the
    // chain `vm.Current.Id` rides), the same as a record's positional property.
    // The head `User` resolves in scope to the declaring class `App.User`.
    assert_eq!(
        index.field_type_name("App.VM.Current"),
        Some("App.User"),
        "the synthesized Current property must carry field type User so `vm.Current.Id` types through"
    );
}

// ---------------------------------------------------------------------------
// System.Text.Json source-gen — JsonSerializerContext
//
// `[JsonSerializable(typeof(User))] partial class AppJsonContext : JsonSerializerContext`
// makes the STJ source generator emit, on the context class:
//   public static AppJsonContext Default { get; }      — self-returning static
//   public JsonTypeInfo<User> User { get; }            — one per [JsonSerializable]
// ---------------------------------------------------------------------------

#[test]
fn json_serializer_context_synthesizes_default_and_per_type_props() {
    let src = "namespace App { public class User { public int Id { get; set; } } [JsonSerializable(typeof(User))] public partial class AppJsonContext : JsonSerializerContext {} }";
    let q = qnames(src);
    assert!(
        q.contains(&"App.AppJsonContext.Default".to_string()),
        "JsonSerializerContext must synthesize the static Default accessor; got {q:?}"
    );
    assert!(
        q.contains(&"App.AppJsonContext.User".to_string()),
        "a [JsonSerializable(typeof(User))] must synthesize a per-type `User` property; got {q:?}"
    );
}

#[test]
fn json_serializer_context_default_is_self_returning() {
    // `Default` returns the context type itself so `AppJsonContext.Default.User`
    // chains; its signature names the context type and it carries a self-return ref.
    let src = "namespace App { public class User { public int Id { get; set; } } [JsonSerializable(typeof(User))] public partial class AppJsonContext : JsonSerializerContext {} }";
    assert_eq!(
        signature_for(src, "App.AppJsonContext.Default").as_deref(),
        Some("AppJsonContext Default"),
        "Default must be typed as the context class"
    );
    assert_eq!(
        return_ref_for(src, "App.AppJsonContext.Default"),
        Some("AppJsonContext".to_string()),
        "Default must carry a self-return ref to the context type"
    );
}

#[test]
fn json_serializer_context_per_type_prop_is_json_type_info() {
    // The per-type property is `JsonTypeInfo<User> User` returning the external
    // `JsonTypeInfo` head (resolves when System.Text.Json is hydrated).
    let src = "namespace App { public class User { public int Id { get; set; } } [JsonSerializable(typeof(User))] public partial class AppJsonContext : JsonSerializerContext {} }";
    assert_eq!(
        signature_for(src, "App.AppJsonContext.User").as_deref(),
        Some("JsonTypeInfo<User> User"),
        "the per-type prop must be typed `JsonTypeInfo<User>`"
    );
    assert_eq!(
        return_ref_for(src, "App.AppJsonContext.User"),
        Some("JsonTypeInfo".to_string()),
        "the per-type prop must carry a return ref to the external JsonTypeInfo head"
    );
}

#[test]
fn non_context_partial_class_synthesizes_no_json_members() {
    // A partial class that does NOT inherit JsonSerializerContext synthesizes no
    // STJ surface, even when it carries a [JsonSerializable] attribute by mistake.
    let src = "namespace App { public class User { public int Id { get; set; } } [JsonSerializable(typeof(User))] public partial class Plain {} }";
    let q = qnames(src);
    assert!(
        !q.iter().any(|n| n == "App.Plain.Default" || n == "App.Plain.User"),
        "a non-JsonSerializerContext class must synthesize no STJ members; got {q:?}"
    );
}

#[test]
fn json_serializer_context_multiple_serializable_types() {
    // Two [JsonSerializable] attributes → one property per type, deduped.
    let src = "namespace App { public class User {} public class Order {} [JsonSerializable(typeof(User))] [JsonSerializable(typeof(Order))] public partial class AppJsonContext : JsonSerializerContext {} }";
    let q = qnames(src);
    assert!(q.contains(&"App.AppJsonContext.User".to_string()), "User prop missing; got {q:?}");
    assert!(q.contains(&"App.AppJsonContext.Order".to_string()), "Order prop missing; got {q:?}");
}
