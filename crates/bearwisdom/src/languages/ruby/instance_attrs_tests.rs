// Tests for instance_attrs.rs — which declaration owns an `@x = …` target.

use super::super::extract;
use crate::types::{ExtractedSymbol, SymbolKind};

/// Indices of the symbols `owner` carries under `name`.
fn members(symbols: &[ExtractedSymbol], owner: &str, name: &str) -> Vec<usize> {
    let owner_index = symbols.iter().position(|s| {
        s.name == owner && matches!(s.kind, SymbolKind::Class | SymbolKind::Interface)
    });
    symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.parent_index == owner_index && s.name == name)
        .map(|(i, _)| i)
        .collect()
}

#[test]
fn an_instance_variable_is_a_property_of_the_class() {
    let source = "class Svc\n  def initialize\n    @cache = Cache.new\n  end\nend\n";
    let r = extract::extract(source);
    let found = members(&r.symbols, "Svc", "@cache");
    assert_eq!(found.len(), 1, "{:?}", r.symbols);
    let declaration = &r.symbols[found[0]];
    assert_eq!(declaration.kind, SymbolKind::Property);
    assert_eq!(declaration.qualified_name, "Svc.@cache");
}

#[test]
fn a_second_method_assigning_the_same_name_reuses_the_declaration() {
    let source = concat!(
        "class Svc\n",
        "  def initialize\n",
        "    @cache = Cache.new\n",
        "  end\n",
        "  def reset\n",
        "    @cache = Cache.new\n",
        "  end\n",
        "end\n",
    );
    let r = extract::extract(source);
    assert_eq!(
        members(&r.symbols, "Svc", "@cache").len(),
        1,
        "{:?}",
        r.symbols
    );
}

#[test]
fn a_memoizing_assignment_declares_the_member() {
    let source = "class Svc\n  def user\n    @user ||= current_user\n  end\nend\n";
    let r = extract::extract(source);
    assert_eq!(
        members(&r.symbols, "Svc", "@user").len(),
        1,
        "{:?}",
        r.symbols
    );
}

#[test]
fn an_assignment_inside_a_block_declares_on_the_class() {
    let source = concat!(
        "class Svc\n",
        "  def run(items)\n",
        "    items.each do |item|\n",
        "      @last = item\n",
        "    end\n",
        "  end\n",
        "end\n",
    );
    let r = extract::extract(source);
    assert_eq!(
        members(&r.symbols, "Svc", "@last").len(),
        1,
        "{:?}",
        r.symbols
    );
}

#[test]
fn the_reader_macro_and_the_variable_are_separate_members() {
    let source = concat!(
        "class Svc\n",
        "  attr_reader :cache\n",
        "  def initialize\n",
        "    @cache = Cache.new\n",
        "  end\n",
        "end\n",
    );
    let r = extract::extract(source);
    assert_eq!(
        members(&r.symbols, "Svc", "cache").len(),
        1,
        "{:?}",
        r.symbols
    );
    assert_eq!(
        members(&r.symbols, "Svc", "@cache").len(),
        1,
        "{:?}",
        r.symbols
    );
}

#[test]
fn a_top_level_method_declares_no_member() {
    let source = "def run\n  @cache = Cache.new\nend\n";
    let r = extract::extract(source);
    assert!(
        !r.symbols
            .iter()
            .any(|s| s.name == "@cache" && s.kind == SymbolKind::Property),
        "{:?}",
        r.symbols
    );
}

#[test]
fn a_module_method_declares_on_the_module() {
    let source = "module Helpers\n  def setup\n    @state = State.new\n  end\nend\n";
    let r = extract::extract(source);
    let found = members(&r.symbols, "Helpers", "@state");
    assert_eq!(found.len(), 1, "{:?}", r.symbols);
    assert_eq!(r.symbols[found[0]].qualified_name, "Helpers.@state");
}
