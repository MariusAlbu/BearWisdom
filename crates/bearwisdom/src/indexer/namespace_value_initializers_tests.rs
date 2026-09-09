use super::*;

#[test]
fn borrow_initializers_use_exact_source_owners_and_shadowed_reads_not_display_names() {
    let source = "fn f(p:Doc) { let p=(&p); let nested=&&p; let raw=&raw const p; let c=|| { let closed=&p; }; }";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    let binding = |name: &str| graph.declaration_starts[&(source.find(name).unwrap() as u32)];
    let owner = extracted
        .symbols
        .iter()
        .position(|s| s.name == "f")
        .unwrap();
    let ValueExpr::Borrow {
        owner: actual,
        span,
        operand,
        ..
    } = &graph.types.values[&binding("p=(")]
    else {
        panic!("borrow recipe");
    };
    assert_eq!(*actual, owner);
    assert_eq!(&source[span.start as usize..span.end as usize], "&p");
    assert_eq!(
        operand.as_ref(),
        &ValueExpr::Read {
            binding: binding("p:Doc"),
            byte: span.start + 1
        }
    );
    let ValueExpr::Borrow { operand, .. } = &graph.types.values[&binding("nested=")] else {
        panic!("outer borrow");
    };
    assert!(
        matches!(operand.as_ref(), ValueExpr::Borrow { operand, .. } if matches!(operand.as_ref(), ValueExpr::Read { binding: b, .. } if *b == binding("p=(")))
    );
    assert_eq!(graph.types.values[&binding("raw=")], ValueExpr::Unknown);
    assert_eq!(
        graph.types.values[&binding("closed=")],
        ValueExpr::Unknown,
        "closure must not borrow outer physical owner"
    );
}

#[test]
fn place_recipes_bind_field_names_and_operand_positions_before_semantic_evaluation() {
    let source =
        "fn f(p:P,t:(A,B)) { let p=p.r#type; let r=&p.item; let d=*p; let n=t.1; let wrong=-p; }";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    let binding = |token: &str| graph.declaration_starts[&(source.find(token).unwrap() as u32)];
    let ValueExpr::Field {
        name,
        byte,
        operand,
    } = &graph.types.values[&binding("p=p.")]
    else {
        panic!("field");
    };
    assert_eq!(Some(*name), graph.name_id("type"));
    assert_eq!(*byte, source.find("r#type").unwrap() as u32);
    assert_eq!(
        operand.as_ref(),
        &ValueExpr::Read {
            binding: binding("p:P"),
            byte: source.find("p.r#type").unwrap() as u32
        }
    );
    let ValueExpr::Borrow { operand, .. } = &graph.types.values[&binding("r=&")] else {
        panic!("borrow");
    };
    assert!(
        matches!(operand.as_ref(),ValueExpr::Field {name,operand,..} if Some(*name)==graph.name_id("item") && matches!(operand.as_ref(),ValueExpr::Read {binding:b,..} if *b==binding("p=p.")))
    );
    assert!(
        matches!(&graph.types.values[&binding("d=*")],ValueExpr::Dereference {operand} if matches!(operand.as_ref(),ValueExpr::Read {binding:b,..} if *b==binding("p=p.")))
    );
    assert!(matches!(
        &graph.types.values[&binding("n=t.")],
        ValueExpr::TupleIndex { index: 1, .. }
    ));
    assert_eq!(graph.types.values[&binding("wrong=")], ValueExpr::Unknown);
    assert_eq!(graph.types.expressions.len(), 5);
}
