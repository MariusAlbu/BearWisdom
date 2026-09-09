use super::*;
use crate::indexer::resolve::engine::testkit::Lookup;

#[test]
fn physical_type_rows_require_complete_canonical_agreement() {
    use crate::indexer::resolve::engine::testkit::sym;
    let lookup = Lookup::new()
        .with(sym(71, "Model", "Model", "interface", "a.ts"))
        .with(sym(72, "Model", "Model", "interface", "a.ts"));
    assert_eq!(agreed_declaration([], &lookup), None);
    assert_eq!(agreed_declaration([71], &lookup), Some(71));
    assert_eq!(agreed_declaration([71, 71], &lookup), Some(71));
    assert_eq!(agreed_declaration([71, 72], &lookup), None);
    assert_eq!(agreed_declaration([71, 99], &lookup), None);
}

#[test]
fn structural_recipes_keep_same_spelled_nominals_distinct() {
    use crate::indexer::resolve::engine::testkit::sym;
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let inner = graph.add_scope(Some(root), 20, 80, true);
    let name = graph.intern("Model");
    let a = graph.declare(root, name, 0, None);
    let b = graph.declare(inner, name, 20, None);
    graph.attach_symbol(0, a);
    graph.attach_symbol(1, b);
    let mut ids = SymbolIds::default();
    ids.set_rows("a.ts".into(), vec![71, 72]);
    let lookup = Lookup::new()
        .with(sym(71, "Model", "Model", "class", "a.ts"))
        .with(sym(72, "Model", "Model", "class", "a.ts"));
    let arena = TypeArena::new();
    let binder = TypeBinder {
        graph: &graph,
        path: "a.ts",
        ids: &ids,
        lookup: &lookup,
        source: None,
        arena: &arena,
    };
    let recipe = TypeExpr::Function(
        vec![TypeExpr::Declaration(a)],
        Box::new(TypeExpr::Optional(Box::new(TypeExpr::Declaration(b)))),
    );
    assert_eq!(
        arena.get(binder.materialize(&recipe)),
        Type::Function {
            params: vec![arena.decl("Model", 71)],
            return_: arena.intern(Type::Optional(arena.decl("Model", 72))),
        }
    );
}

#[test]
fn missing_type_rows_do_not_reenter_a_name_lookup() {
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("KnownName");
    let binding = graph.declare(scope, name, 0, None);
    graph.attach_symbol(1, binding);
    let mut ids = SymbolIds::default();
    ids.insert_key("a.ts".into(), "KnownName".into(), 99);
    ids.set_rows("a.ts".into(), vec![1, 0]);
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let binder = TypeBinder {
        graph: &graph,
        path: "a.ts",
        ids: &ids,
        lookup: &lookup,
        source: None,
        arena: &arena,
    };
    assert_eq!(
        arena.get(binder.materialize(&TypeExpr::Declaration(binding))),
        Type::Unknown
    );
    let recipe = TypeExpr::Function(vec![], Box::new(TypeExpr::Declaration(binding)));
    assert_eq!(
        arena.get(binder.materialize(&recipe)),
        Type::Function {
            params: vec![],
            return_: arena.intern(Type::Unknown),
        }
    );
}
