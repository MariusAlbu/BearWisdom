use super::*;
use crate::types::EdgeKind;

/// An RSpec suite: every statement sits inside a block, so the extractor
/// declares nothing while the file references a project class and its method.
const SPEC_SUITE: &str = r#"require 'widget'

describe Widget do
  it 'spins' do
    Widget.new.spin
  end
end
"#;

/// A ruby file that declares a class — the shape the pass must leave alone.
const DECLARING: &str = r#"class Widget
  def spin
    Gear.turn
  end
end
"#;

fn extract_ruby(source: &str, path: &str) -> ExtractionResult {
    use crate::languages::LanguagePlugin;
    crate::languages::default_registry()
        .get("ruby")
        .extract(source, path, "ruby")
}

/// `(target, kind)` for every ref, for assertion messages.
fn ref_shapes(result: &ExtractionResult) -> Vec<(String, EdgeKind)> {
    result
        .refs
        .iter()
        .map(|r| (r.target_name.clone(), r.kind))
        .collect()
}

#[test]
fn a_file_of_blocks_gets_one_owner_that_carries_its_refs() {
    let mut result = extract_ruby(SPEC_SUITE, "spec/widget_spec.rb");
    assert!(
        result.symbols.is_empty(),
        "a block-only suite declares nothing: {:?}",
        result.symbols
    );
    assert!(!result.refs.is_empty(), "the suite references Widget");

    assert!(materialize(&mut result, "spec/widget_spec.rb", 7));

    assert_eq!(result.symbols.len(), 1, "exactly one owner");
    let owner = &result.symbols[0];
    assert_eq!(owner.name, "widget_spec");
    assert_eq!(owner.qualified_name, "spec/widget_spec");
    assert_eq!(owner.kind, SymbolKind::Module);
    assert_eq!(owner.visibility, Some(Visibility::Private));
    assert_eq!(owner.parent_index, None);
    assert_eq!(owner.scope_path, None);
    assert_eq!((owner.start_line, owner.end_line), (0, 6));

    assert!(
        result.refs.iter().all(|r| r.source_symbol_index == 0),
        "every ref is attributed to the owner"
    );
    assert!(
        result
            .refs
            .iter()
            .any(|r| r.target_name == "Widget" && r.kind == EdgeKind::TypeRef),
        "the referenced class survives: {:?}",
        ref_shapes(&result)
    );
    assert!(
        result.refs.iter().any(|r| r.target_name == "spin"),
        "the called method survives: {:?}",
        ref_shapes(&result)
    );
}

#[test]
fn a_file_with_nothing_to_own_gets_no_owner() {
    let mut result = extract_ruby("# a comment and nothing else\n", "spec/empty_spec.rb");
    assert!(result.symbols.is_empty());
    assert!(result.refs.is_empty(), "{:?}", ref_shapes(&result));

    assert!(!materialize(&mut result, "spec/empty_spec.rb", 1));
    assert!(result.symbols.is_empty(), "no owner without links to own");
}

#[test]
fn a_file_that_declares_a_symbol_is_untouched() {
    let mut result = extract_ruby(DECLARING, "lib/widget.rb");
    let symbols_before: Vec<String> = result.symbols.iter().map(|s| s.name.clone()).collect();
    let refs_before: Vec<(String, usize)> = result
        .refs
        .iter()
        .map(|r| (r.target_name.clone(), r.source_symbol_index))
        .collect();
    assert!(!symbols_before.is_empty(), "the class is declared");

    assert!(!materialize(&mut result, "lib/widget.rb", 5));

    let symbols_after: Vec<String> = result.symbols.iter().map(|s| s.name.clone()).collect();
    let refs_after: Vec<(String, usize)> = result
        .refs
        .iter()
        .map(|r| (r.target_name.clone(), r.source_symbol_index))
        .collect();
    assert_eq!(symbols_before, symbols_after);
    assert_eq!(refs_before, refs_after);
}
