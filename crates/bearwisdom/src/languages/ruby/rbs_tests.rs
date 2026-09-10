use crate::languages::LanguagePlugin;
use crate::type_checker::core::types::{Type, TypeArena};
use crate::types::SymbolKind;

#[test]
fn rbs_method_with_required_block_becomes_structural_callback_slot() {
    let source = r#"
module Services
  class Catalog
    def visit: (Request) { (Item, Context) -> Result } -> void
  end
end
"#;
    let plugin = super::RubyPlugin;
    let mut extracted = plugin.extract(source, "catalog.rbs", "rbs");
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Services::Catalog::visit")
        .expect("strict RBS method contract");
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.scope_path.as_deref(), Some("Services::Catalog"));
    assert_eq!(
        method.signature.as_deref(),
        Some("visit(arg0: Request, callback: (Item, Context) -> Result): void")
    );

    let arena = TypeArena::new();
    crate::languages::common::populate_return_type_ids(&mut extracted, &arena, "rbs");
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Services::Catalog::visit")
        .unwrap();
    assert!(matches!(
        arena.get(method.param_types[1]),
        Type::Function { params, return_ }
            if params.len() == 2
                && matches!(arena.get(params[0]), Type::Class(name) if name == "Item")
                && matches!(arena.get(params[1]), Type::Class(name) if name == "Context")
                && matches!(arena.get(return_), Type::Class(name) if name == "Result")
    ));
}

#[test]
fn rbs_rejects_non_structural_and_overloaded_contracts() {
    let source = r#"
class Catalog
  def optional: () ?{ (Item) -> void } -> void
  def generic: [T] () { (T) -> void } -> void
  def union: () { (Item | Other) -> void } -> void
  def keys: (name: String) { (Item) -> void } -> void
  def overload: () { (Item) -> void } -> void
  def overload: () { (Other) -> void } -> void
  def broken: () { (Item) -> void -> void
end
"#;
    let extracted = super::rbs::extract(source);
    assert!(
        extracted
            .symbols
            .iter()
            .all(|symbol| symbol.kind != SymbolKind::Method),
        "unsupported RBS shapes must not produce callable contracts: {:?}",
        extracted
            .symbols
            .iter()
            .map(|symbol| &symbol.qualified_name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn rbs_rejects_contracts_when_scope_structure_is_unbalanced() {
    for source in [
        r#"
end
class Catalog
  def visit: () { (Item) -> void } -> void
end
"#,
        r#"
class Catalog
  def visit: () { (Item) -> void } -> void
"#,
        r#"
interface Unsupported[T]
  class Catalog
    def visit: () { (Item) -> void } -> void
  end
"#,
    ] {
        let extracted = super::rbs::extract(source);
        assert!(
            extracted
                .symbols
                .iter()
                .all(|symbol| symbol.kind != SymbolKind::Method),
            "unbalanced RBS scopes must not materialize callable contracts: {:?}",
            extracted
                .symbols
                .iter()
                .map(|symbol| &symbol.qualified_name)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn rbs_route_is_dedicated_and_does_not_offer_ruby_grammar() {
    let plugin = super::RubyPlugin;
    assert_eq!(plugin.language_id_for_extension(".rbs"), Some("rbs"));
    assert!(plugin.grammar("rbs").is_none());
    assert!(plugin.grammar("ruby").is_some());
    assert_eq!(
        crate::languages::default_registry().language_by_extension("catalog.rbs"),
        Some("rbs")
    );
}

#[test]
fn rbi_dsl_does_not_form_an_rbs_contract() {
    let source = r#"
class Catalog
  sig { params(block: T.proc.params(item: Item).void).void }
  def each(&block); end
end
"#;
    let extracted = super::rbs::extract(source);
    assert!(extracted
        .symbols
        .iter()
        .all(|symbol| symbol.kind != SymbolKind::Method));
    assert_eq!(
        crate::languages::default_registry().language_by_extension("catalog.rbi"),
        None,
        "RBI remains deliberately unsupported by the RBS route"
    );
}
