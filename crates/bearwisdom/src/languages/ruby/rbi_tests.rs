use crate::type_checker::core::types::{Type, TypeArena};
use crate::types::SymbolKind;

#[test]
fn rbi_nested_scopes_emit_a_structural_callback_contract() {
    let source = r#"
module Services
  class Catalog
    sig { params(request: Request, block: T.proc.params(item: Item).returns(Result)).returns(Response) }
    def visit(request, &block); end
  end
end
"#;
    let extracted = super::rbi::extract(source);
    let module = extracted
        .symbols
        .iter()
        .position(|symbol| symbol.qualified_name == "Services")
        .expect("module symbol");
    let class = extracted
        .symbols
        .iter()
        .position(|symbol| symbol.qualified_name == "Services::Catalog")
        .expect("class symbol");
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Services::Catalog::visit")
        .expect("strict RBI method contract");
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.parent_index, Some(class));
    assert_eq!(extracted.symbols[class].parent_index, Some(module));
    assert_eq!(method.scope_path.as_deref(), Some("Services::Catalog"));
    assert_eq!(method.start_line, 4);
    assert_eq!(method.start_col, 8);
    assert_eq!(
        method.signature.as_deref(),
        Some("visit(request: Request, &block: (Item) -> Result): Response")
    );
}

#[test]
fn rbi_void_callback_and_method_return_are_structural() {
    let source = r#"
class Catalog
  sig { params(block: T.proc.void).void }
  def each(&block); end
end
"#;
    let mut extracted = super::rbi::extract(source);
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Catalog::each")
        .expect("RBI method");
    assert_eq!(
        method.signature.as_deref(),
        Some("each(&block: () -> void): void")
    );

    let arena = TypeArena::new();
    crate::languages::common::populate_return_type_ids(&mut extracted, &arena, "rbi");
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Catalog::each")
        .unwrap();
    assert!(matches!(
        arena.get(method.param_types[0]),
        Type::Function { params, return_ }
            if params.is_empty() && matches!(arena.get(return_), Type::Class(name) if name == "void")
    ));
}

#[test]
fn rbi_required_positional_proc_is_a_structural_callback_contract() {
    let source = r#"
class Catalog
  sig { params(callback: T.proc.params(item: Item).returns(Result)).returns(Response) }
  def visit(callback); end
end
"#;
    let mut extracted = super::rbi::extract(source);
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Catalog::visit")
        .expect("strict positional RBI method contract");
    assert_eq!(
        method.signature.as_deref(),
        Some("visit(^callback: (Item) -> Result): Response")
    );

    let arena = TypeArena::new();
    crate::languages::common::populate_return_type_ids(&mut extracted, &arena, "rbi");
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Catalog::visit")
        .unwrap();
    assert!(matches!(
        arena.get(method.param_types[0]),
        Type::Function { params, return_ }
            if matches!(arena.get(params[0]), Type::Class(name) if name == "Item")
                && matches!(arena.get(return_), Type::Class(name) if name == "Result")
    ));
}

#[test]
fn rbi_required_positional_void_proc_is_structural() {
    let source = r#"
class Catalog
  sig { params(callback: T.proc.void).void }
  def each(callback); end
end
"#;
    let mut extracted = super::rbi::extract(source);
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Catalog::each")
        .expect("strict positional RBI void contract");
    assert_eq!(
        method.signature.as_deref(),
        Some("each(^callback: () -> void): void")
    );
    let arena = TypeArena::new();
    crate::languages::common::populate_return_type_ids(&mut extracted, &arena, "rbi");
    let method = extracted
        .symbols
        .iter()
        .find(|symbol| symbol.qualified_name == "Catalog::each")
        .unwrap();
    assert!(matches!(
        arena.get(method.param_types[0]),
        Type::Function { params, return_ }
            if params.is_empty() && matches!(arena.get(return_), Type::Class(name) if name == "void")
    ));
}

#[test]
fn rbi_rejects_non_exact_positional_proc_contract_shapes() {
    let cases = [
        (
            "missing positional parameter",
            "sig { params(callback: T.proc.void).void }\n  def each; end",
        ),
        (
            "additional ordinary parameter",
            "sig { params(value: Value, callback: T.proc.void).void }\n  def each(value, callback); end",
        ),
        (
            "attached block alongside callback",
            "sig { params(callback: T.proc.void, block: T.proc.void).void }\n  def each(callback, &block); end",
        ),
        (
            "optional positional parameter",
            "sig { params(callback: T.proc.void).void }\n  def each(callback = nil); end",
        ),
        (
            "keyword positional parameter",
            "sig { params(callback: T.proc.void).void }\n  def each(callback:); end",
        ),
        (
            "keyrest positional parameter",
            "sig { params(callback: T.proc.void).void }\n  def each(**callback); end",
        ),
        (
            "splat positional parameter",
            "sig { params(callback: T.proc.void).void }\n  def each(*callback); end",
        ),
        (
            "destructured positional parameter",
            "sig { params(callback: T.proc.void).void }\n  def each((callback)); end",
        ),
        (
            "parameter name mismatch",
            "sig { params(callback: T.proc.void).void }\n  def each(handler); end",
        ),
        (
            "ordinary parameter is not a proc",
            "sig { params(callback: Callback).void }\n  def each(callback); end",
        ),
        (
            "proc option",
            "sig { params(callback: T.proc.bind(Context).void).void }\n  def each(callback); end",
        ),
        (
            "generic proc input",
            "sig { params(callback: T.proc.params(item: T::Array[Item]).void).void }\n  def each(callback); end",
        ),
        (
            "empty params proc spelling",
            "sig { params(callback: T.proc.params().void).void }\n  def each(callback); end",
        ),
    ];

    for (label, declaration) in cases {
        let source = format!("class Catalog\n  {declaration}\nend\n");
        let extracted = super::rbi::extract(&source);
        assert!(
            extracted
                .symbols
                .iter()
                .all(|symbol| symbol.qualified_name != "Catalog::each"),
            "{label} must not produce an RBI contract: {:?}",
            extracted
                .symbols
                .iter()
                .map(|symbol| &symbol.qualified_name)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn rbi_rejects_every_non_exact_contract_shape() {
    let cases = [
        (
            "do-end sig",
            r#"class Catalog
  sig do
    params(block: T.proc.void).void
  end
  def each(&block); end
end"#,
        ),
        (
            "sig options",
            r#"class Catalog
  sig(:final) { params(block: T.proc.void).void }
  def each(&block); end
end"#,
        ),
        (
            "sig block parameter",
            r#"class Catalog
  sig { |x| params(block: T.proc.void).void }
  def each(&block); end
end"#,
        ),
        (
            "multiple sig expressions",
            r#"class Catalog
  sig { params(block: T.proc.void).void; nil }
  def each(&block); end
end"#,
        ),
        (
            "adjacent signatures form an overload",
            r#"class Catalog
  sig { params(block: T.proc.void).void }
  sig { params(block: T.proc.void).void }
  def each(&block); end
end"#,
        ),
        (
            "receiver sig",
            r#"class Catalog
  T.sig { params(block: T.proc.void).void }
  def each(&block); end
end"#,
        ),
        (
            "not adjacent",
            r#"class Catalog
  sig { params(block: T.proc.void).void }
  private
  def each(&block); end
end"#,
        ),
        (
            "singleton method",
            r#"class Catalog
  sig { params(block: T.proc.void).void }
  def self.each(&block); end
end"#,
        ),
        (
            "optional method parameter",
            r#"class Catalog
  sig { params(name: Name, block: T.proc.void).void }
  def each(name = nil, &block); end
end"#,
        ),
        (
            "keyword method parameter",
            r#"class Catalog
  sig { params(name: Name, block: T.proc.void).void }
  def each(name:, &block); end
end"#,
        ),
        (
            "keyrest method parameter",
            r#"class Catalog
  sig { params(name: Name, block: T.proc.void).void }
  def each(**name, &block); end
end"#,
        ),
        (
            "splat method parameter",
            r#"class Catalog
  sig { params(name: Name, block: T.proc.void).void }
  def each(*name, &block); end
end"#,
        ),
        (
            "forward method parameter",
            r#"class Catalog
  sig { params(block: T.proc.void).void }
  def each(..., &block); end
end"#,
        ),
        (
            "destructured method parameter",
            r#"class Catalog
  sig { params(value: Value, block: T.proc.void).void }
  def each((value), &block); end
end"#,
        ),
        (
            "multiple callback parameters",
            r#"class Catalog
  sig { params(block: T.proc.void).void }
  def each(&first, &block); end
end"#,
        ),
        (
            "anonymous block parameter",
            r#"class Catalog
  sig { params(block: T.proc.void).void }
  def each(&); end
end"#,
        ),
        (
            "parameter name mismatch",
            r#"class Catalog
  sig { params(value: Name, block: T.proc.void).void }
  def each(name, &block); end
end"#,
        ),
        (
            "generic type",
            r#"class Catalog
  sig { params(items: T::Array[Item], block: T.proc.void).void }
  def each(items, &block); end
end"#,
        ),
        (
            "nilable type",
            r#"class Catalog
  sig { params(item: T.nilable(Item), block: T.proc.void).void }
  def each(item, &block); end
end"#,
        ),
        (
            "untyped type",
            r#"class Catalog
  sig { params(item: T.untyped, block: T.proc.void).void }
  def each(item, &block); end
end"#,
        ),
        (
            "non proc callback",
            r#"class Catalog
  sig { params(block: Callback).void }
  def each(&block); end
end"#,
        ),
        (
            "proc option",
            r#"class Catalog
  sig { params(block: T.proc.bind(Context).void).void }
  def each(&block); end
end"#,
        ),
        (
            "callback parameters need simple values",
            r#"class Catalog
  sig { params(block: T.proc.params(item: T.nilable(Item)).void).void }
  def each(&block); end
end"#,
        ),
        (
            "empty proc params must use the zero-arity form",
            r#"class Catalog
  sig { params(block: T.proc.params().void).void }
  def each(&block); end
end"#,
        ),
        (
            "qualified class declaration",
            r#"class Services::Catalog
  sig { params(block: T.proc.void).void }
  def each(&block); end
end"#,
        ),
        (
            "malformed ruby",
            r#"class Catalog
  sig { params(block: T.proc.void).void }
  def each(&block)
end"#,
        ),
    ];

    for (label, source) in cases {
        let extracted = super::rbi::extract(source);
        assert!(
            extracted
                .symbols
                .iter()
                .all(|symbol| symbol.kind != SymbolKind::Method),
            "{label} must not produce an RBI contract: {:?}",
            extracted
                .symbols
                .iter()
                .map(|symbol| &symbol.qualified_name)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn rbi_suppresses_duplicate_qualified_methods() {
    let source = r#"
class Catalog
  sig { params(block: T.proc.void).void }
  def each(&block); end

  sig { params(block: T.proc.void).void }
  def each(&block); end
end
"#;
    let extracted = super::rbi::extract(source);
    assert!(extracted
        .symbols
        .iter()
        .all(|symbol| symbol.qualified_name != "Catalog::each"));
}
