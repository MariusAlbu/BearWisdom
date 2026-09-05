use super::{binding_symbol, BindingSymbols};
use crate::indexer::flow::run_flow_queries;
use crate::languages::default_registry;
use crate::types::{ExtractedSymbol, FlowMeta, SymbolKind};

// ---------------------------------------------------------------------------
// Unit: correlation and synthesis rules
// ---------------------------------------------------------------------------

fn decl(name: &str, kind: SymbolKind, start: u32, end: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: None,
        start_line: start,
        end_line: end,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

/// Parse `src` as TypeScript and hand the first identifier node named `text`
/// to `f`.
fn with_identifier<R>(src: &str, text: &str, f: impl FnOnce(&tree_sitter::Node) -> R) -> R {
    let plugin = default_registry().get_dedicated("typescript").expect("ts plugin");
    let grammar = plugin.grammar("typescript").expect("ts grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).expect("set language");
    let tree = parser.parse(src, None).expect("parse");
    let mut stack = vec![tree.root_node()];
    let mut first: Option<tree_sitter::Node> = None;
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier" && n.utf8_text(src.as_bytes()).ok() == Some(text) {
            if first.is_none_or(|f| n.start_byte() < f.start_byte()) {
                first = Some(n);
            }
        }
        let mut c = n.walk();
        for ch in n.named_children(&mut c) {
            stack.push(ch);
        }
    }
    f(&first.unwrap_or_else(|| panic!("no identifier `{text}` in snippet")))
}

#[test]
fn uncorrelated_binding_is_synthesized_under_the_enclosing_declaration() {
    let src = "function f(x: Foo) {\n  return x;\n}\n";
    let mut symbols = vec![decl("f", SymbolKind::Function, 0, 2)];
    let idx = with_identifier(src, "x", |n| {
        binding_symbol("x", n, SymbolKind::Parameter, &mut symbols, BindingSymbols::Synthesize)
    });
    assert_eq!(idx, Some(1));
    let sym = &symbols[1];
    assert_eq!(sym.kind, SymbolKind::Parameter);
    assert_eq!(sym.qualified_name, "f.x");
    assert_eq!(sym.parent_index, Some(0));
    assert_eq!(sym.scope_path.as_deref(), Some("f"));
    assert_eq!(sym.start_line, 0);
}

/// A parameter correlates only with a symbol on its own line: a same-named
/// field declared earlier is not the parameter.
#[test]
fn parameter_does_not_correlate_with_an_earlier_same_name_symbol() {
    let src = "class C {\n  x: Bar;\n  m(x: Foo) {\n    return x;\n  }\n}\n";
    let mut symbols = vec![
        decl("C", SymbolKind::Class, 0, 5),
        decl("x", SymbolKind::Field, 1, 1),
        decl("m", SymbolKind::Method, 2, 4),
    ];
    let idx = with_identifier(src, "x", |first| {
        // The first `x` identifier is the field; find the parameter's.
        let _ = first;
        let plugin = default_registry().get_dedicated("typescript").unwrap();
        let grammar = plugin.grammar("typescript").unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar).unwrap();
        let tree = parser.parse(src, None).unwrap();
        let mut stack = vec![tree.root_node()];
        let mut param: Option<tree_sitter::Node> = None;
        while let Some(n) = stack.pop() {
            if n.kind() == "required_parameter" {
                param = n.child_by_field_name("pattern");
                break;
            }
            let mut c = n.walk();
            for ch in n.named_children(&mut c) {
                stack.push(ch);
            }
        }
        binding_symbol(
            "x",
            &param.expect("parameter node"),
            SymbolKind::Parameter,
            &mut symbols,
            BindingSymbols::Synthesize,
        )
    });
    assert_eq!(idx, Some(3), "the parameter gets its own symbol");
    assert_eq!(symbols[3].qualified_name, "m.x");
    assert_eq!(symbols[3].parent_index, Some(2));
}

/// A binding outside every declaration gets no symbol: in a file whose
/// extractor emitted nothing, refs carry source index 0, and a synthesized
/// file-level binding would become the source of all of them.
#[test]
fn binding_outside_every_declaration_is_not_synthesized() {
    let src = "const x: Foo = make();
";
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let idx = with_identifier(src, "x", |n| {
        binding_symbol("x", n, SymbolKind::Variable, &mut symbols, BindingSymbols::Synthesize)
    });
    assert_eq!(idx, None);
    assert!(symbols.is_empty());
}

#[test]
fn correlate_only_skips_an_uncorrelated_binding() {
    let src = "function f(x: Foo) {}\n";
    let mut symbols = vec![decl("f", SymbolKind::Function, 0, 0)];
    let idx = with_identifier(src, "x", |n| {
        binding_symbol("x", n, SymbolKind::Parameter, &mut symbols, BindingSymbols::CorrelateOnly)
    });
    assert_eq!(idx, None);
    assert_eq!(symbols.len(), 1);
}

// ---------------------------------------------------------------------------
// Per-language: the flow queries name parameters and typed locals, and the
// runner gives each a symbol with its declared type seeded
// ---------------------------------------------------------------------------

fn flow_for(lang: &str, ext: &str, src: &str) -> (Vec<ExtractedSymbol>, FlowMeta) {
    let plugin = default_registry()
        .get_dedicated(lang)
        .unwrap_or_else(|| panic!("plugin {lang}"));
    let result = plugin.extract(src, &format!("t.{ext}"), lang);
    let grammar = plugin.grammar(lang).unwrap_or_else(|| panic!("grammar {lang}"));
    let cfg = plugin.flow_config().unwrap_or_else(|| panic!("flow config {lang}"));
    let mut symbols = result.symbols;
    let mut refs = result.refs;
    let meta = run_flow_queries(src, &grammar, cfg, &mut symbols, &mut refs, BindingSymbols::Synthesize);
    (symbols, meta)
}

/// The declared types seeded for every symbol named `name`, in symbol order —
/// the extractor's own binding when it emitted one (whatever kind it chose),
/// else the synthesized one. Empty when no symbol carries the name.
fn seeded_types(lang: &str, ext: &str, src: &str, name: &str) -> Vec<Option<String>> {
    let (symbols, meta) = flow_for(lang, ext, src);
    let out: Vec<Option<String>> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.name == name)
        .map(|(i, _)| meta.flow_binding_decl_type.get(&i).cloned())
        .collect();
    assert!(
        !out.is_empty(),
        "{lang}: no symbol named `{name}`; symbols = {:?}",
        symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect::<Vec<_>>()
    );
    out
}

/// `name` has a binding whose seeded declared type is `ty`.
fn assert_typed(lang: &str, ext: &str, src: &str, name: &str, ty: &str) {
    let types = seeded_types(lang, ext, src, name);
    assert!(
        types.iter().any(|t| t.as_deref() == Some(ty)),
        "{lang}: `{name}` should seed `{ty}`, got {types:?}"
    );
}

/// `name` has a binding and none of its bindings seeds a type.
fn assert_untyped(lang: &str, ext: &str, src: &str, name: &str) {
    let types = seeded_types(lang, ext, src, name);
    assert!(types.iter().all(|t| t.is_none()), "{lang}: `{name}` should seed nothing, got {types:?}");
}

/// The synthesized kind for a binding the extractor never emitted.
fn synthesized_kind(lang: &str, ext: &str, src: &str, name: &str) -> SymbolKind {
    let (symbols, _) = flow_for(lang, ext, src);
    symbols
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("{lang}: no symbol named `{name}`"))
        .kind
}

#[test]
fn typescript_parameter_and_local() {
    let src = "function f(text: string, n) {
  const y: Foo = g();
  return text.length;
}
";
    assert_typed("typescript", "ts", src, "text", "string");
    assert_untyped("typescript", "ts", src, "n");
    assert_eq!(synthesized_kind("typescript", "ts", src, "n"), SymbolKind::Parameter);
    assert_typed("typescript", "ts", src, "y", "Foo");
}

#[test]
fn javascript_parameter_is_synthesized() {
    let src = "function f(x) {
  return x;
}
";
    assert_untyped("javascript", "js", src, "x");
    assert_eq!(synthesized_kind("javascript", "js", src, "x"), SymbolKind::Parameter);
}

#[test]
fn csharp_parameter_and_typed_local() {
    let src = "class C {
  void M(Foo x) {
    Foo y = x;
    var z = y;
  }
}
";
    assert_typed("csharp", "cs", src, "x", "Foo");
    assert_typed("csharp", "cs", src, "y", "Foo");
    assert_untyped("csharp", "cs", src, "z");
}

#[test]
fn java_parameter_and_typed_local() {
    let src = "class C {
  void m(Foo x) {
    Foo y = x;
    var z = y;
  }
}
";
    assert_typed("java", "java", src, "x", "Foo");
    assert_typed("java", "java", src, "y", "Foo");
    assert_untyped("java", "java", src, "z");
}

#[test]
fn kotlin_parameter_and_typed_val() {
    let src = "fun f(x: Foo) {
    val y: Foo = x
    val z = y
}
";
    assert_typed("kotlin", "kt", src, "x", "Foo");
    assert_typed("kotlin", "kt", src, "y", "Foo");
}

#[test]
fn scala_parameter_lambda_binding_and_typed_val() {
    let src = "object O {
  def f(x: Foo): Unit = {
    val y: Foo = x
    List(1).map(n => n)
  }
}
";
    assert_typed("scala", "scala", src, "x", "Foo");
    assert_typed("scala", "scala", src, "y", "Foo");
    assert_untyped("scala", "scala", src, "n");
    assert_eq!(synthesized_kind("scala", "scala", src, "n"), SymbolKind::Parameter);
}

#[test]
fn rust_parameter_and_closure_parameter() {
    let src = "fn f(x: Foo) {
    let y: Foo = x;
    let g = |n| n;
}
";
    assert_typed("rust", "rs", src, "x", "Foo");
    assert_typed("rust", "rs", src, "y", "Foo");
    assert_untyped("rust", "rs", src, "n");
}

#[test]
fn php_parameter_and_promoted_property() {
    let src = "<?php
class C {
  public function __construct(private Bar $b) {}
  function f(Foo $x, $u) {
    $x->run();
  }
}
";
    assert_typed("php", "php", src, "x", "Foo");
    assert_untyped("php", "php", src, "u");
    assert_typed("php", "php", src, "b", "Bar");
}

#[test]
fn dart_parameter_and_typed_local() {
    let src = "void f(Foo x, y) {
  Foo z = x;
}
";
    assert_typed("dart", "dart", src, "x", "Foo");
    assert_untyped("dart", "dart", src, "y");
    assert_eq!(synthesized_kind("dart", "dart", src, "y"), SymbolKind::Parameter);
    assert_typed("dart", "dart", src, "z", "Foo");
}

#[test]
fn go_parameter_and_typed_var() {
    let src = "package p

func f(x Foo) {
	var y Foo = x
	_ = y
}
";
    assert_typed("go", "go", src, "x", "Foo");
    assert_typed("go", "go", src, "y", "Foo");
}

#[test]
fn python_typed_and_untyped_parameters() {
    let src = "def f(x: Foo, y, z: Bar = None):
    return x
";
    assert_typed("python", "py", src, "x", "Foo");
    assert_untyped("python", "py", src, "y");
    assert_typed("python", "py", src, "z", "Bar");
}
