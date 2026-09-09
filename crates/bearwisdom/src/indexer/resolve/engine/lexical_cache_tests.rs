use super::*;

#[test]
fn source_value_copy_reads_the_rhs_binding_at_its_own_position() {
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("p");
    let source = graph.declare_ordered(scope, name, 10, None);
    let copy = graph.declare_ordered(scope, name, 30, None);
    graph.types.values.insert(
        copy,
        crate::indexer::lexical::type_syntax::ValueExpr::Read {
            binding: source,
            byte: 20,
        },
    );
    let cache = LexicalCache::new(&graph, &arena);
    let before = arena.decl("Same", 71);
    let after = arena.decl("Same", 72);
    cache.set_cursor(11);
    cache.record(source, before, false);
    cache.set_cursor(40);
    cache.record(source, after, false);
    assert_eq!(cache.type_at(copy, 60), Some(before));
    assert_eq!(cache.type_at(source, 60), Some(after));
    cache.clear();
    assert_eq!(cache.type_at(copy, 60), None);
}

#[test]
fn local_borrow_recipe_reads_prior_facts_without_moving_cursor_or_aliasing_later_writes() {
    use crate::indexer::lexical::type_syntax::ValueExpr;
    use crate::type_checker::core::types::{Indirection, Lifetime, Mutability};
    use crate::types::SourceSpan;
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let source_name = graph.intern("p");
    let local_name = graph.intern("r");
    let source = graph.declare_ordered(scope, source_name, 5, None);
    let local = graph.declare_ordered(scope, local_name, 30, None);
    let span = SourceSpan { start: 20, end: 22 };
    let expr = ValueExpr::Borrow {
        owner: 1,
        span,
        mutability: Mutability::Shared,
        operand: Box::new(ValueExpr::Read {
            binding: source,
            byte: 21,
        }),
    };
    graph.types.values.insert(local, expr.clone());
    let mut cache = LexicalCache::new(&graph, &arena);
    assert_eq!(
        cache.type_at(local, 60),
        Some(arena.intern(Type::Unknown)),
        "uninstalled owner is not evidence"
    );
    cache
        .value_recipes
        .insert(local, values::lower(&expr, &|_| Some(99), &|_| None, 0));
    let before = arena.decl("Same", 71);
    let later = arena.decl("Same", 72);
    cache.set_cursor(10);
    cache.record(source, before, false);
    cache.set_cursor(50);
    cache.record(source, later, false);
    let expected = arena.intern(Type::Indirect {
        kind: Indirection::Reference(Lifetime::Inference {
            owner: 99,
            byte: 20,
        }),
        mutability: Mutability::Shared,
        inner: before,
    });
    assert_eq!(cache.type_at(local, 60), Some(expected));
    assert_eq!(cache.type_at(source, 60), Some(later));
    assert_eq!(cache.cursor.get(), 50);
    cache.clear();
    assert_eq!(cache.type_at(local, 60), Some(arena.intern(Type::Unknown)));
}

#[test]
fn cyclic_value_recipes_stop_at_the_combined_binding_expression_depth_limit() {
    use crate::indexer::lexical::type_syntax::ValueExpr;
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("p");
    let binding = graph.declare_ordered(scope, name, 0, None);
    graph
        .types
        .values
        .insert(binding, ValueExpr::Read { binding, byte: 10 });
    assert_eq!(LexicalCache::new(&graph, &arena).type_at(binding, 20), None);
}

#[test]
fn failed_inference_replaces_a_previous_flow_fact_with_explicit_unknown() {
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("value");
    let binding = graph.declare_ordered(root, name, 5, None);
    let cache = LexicalCache::new(&graph, &arena);
    cache.set_cursor(10);
    cache.record(binding, arena.decl("Doc", 41), true);
    cache.set_cursor(20);
    cache.record_cause(
        binding,
        Cause::new(Some(42), super::super::cause::CauseKind::UncapturedReturn),
    );
    cache.set_cursor(30);
    assert_eq!(cache.local_type("value"), Some(arena.intern(Type::Unknown)));
}

#[test]
fn argument_fact_and_callable_reads_do_not_borrow_later_versions() {
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("value");
    let binding = graph.declare(root, name, 1, None);
    let span = crate::types::SourceSpan { start: 20, end: 25 };
    graph.argument_reads.insert(span, binding);
    let cache = LexicalCache::new(&graph, &arena);
    let first = arena.decl("Same", 71);
    let later = arena.decl("Same", 72);
    cache.set_cursor(10);
    cache.record(binding, first, false);
    cache.record_callable(binding, 31);
    cache.set_cursor(70);
    cache.record(binding, later, false);
    cache.record_callable(binding, 32);
    assert_eq!(
        cache.argument_reference(span).unwrap().value_type,
        Some(first)
    );
    assert_eq!(cache.argument_reference(span).unwrap().callable, Some(31));
    assert_eq!(cache.local_type("value"), Some(later));
    assert_eq!(cache.callable("value"), Some(32));
}

#[test]
fn reference_targets_are_positional_and_missing_rows_remain_missing() {
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("callback");
    let binding = graph.declare(scope, name, 1, Some("() => void".into()));
    graph.attach_symbol(1, binding);
    graph.references.insert(25, binding);
    let mut cache = LexicalCache::new(&graph, &arena);
    let mut ids = crate::indexer::symbol_ids::SymbolIds::default();
    ids.insert_key("a.ts".into(), "f.callback".into(), 99);
    ids.set_rows("a.ts".into(), vec![10, 0]);
    cache.install_declarations("a.ts", &ids);
    cache.set_cursor(25);
    assert_eq!(cache.reference(25).unwrap().declaration, None);
    ids.set_rows("a.ts".into(), vec![10, 20]);
    cache.install_declarations("a.ts", &ids);
    assert_eq!(cache.reference(25).unwrap().declaration, Some(20));
    assert!(cache.reference(26).is_none());
    cache.clear();
    cache.set_cursor(25);
    assert_eq!(cache.reference(25).unwrap().declaration, Some(20));
}

#[test]
fn nested_function_union_fact_cannot_overwrite_outer_execution_state() {
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    graph.add_scope(Some(root), 20, 40, true);
    let name = graph.intern("value");
    let binding = graph.declare(root, name, 1, Some("Alpha | Beta".into()));
    graph.preserve_non_union_type = true;
    let cache = LexicalCache::new(&graph, &arena);
    let alpha = arena.class("Alpha");
    let beta = arena.class("Beta");
    cache.set_cursor(10);
    cache.record(binding, alpha, true);
    assert_eq!(cache.local_type("value"), Some(alpha));
    cache.set_cursor(30);
    cache.record(binding, beta, false);
    assert_eq!(cache.local_type("value"), Some(beta));
    cache.set_cursor(50);
    assert_eq!(cache.local_type("value"), Some(alpha));
}

#[test]
fn facts_keep_canonical_variants_and_do_not_reparse_display_strings() {
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("value");
    let binding = graph.declare(root, name, 1, None);
    let cache = LexicalCache::new(&graph, &arena);
    let ty = arena.intern(Type::Optional(
        arena.primitive(crate::type_checker::core::types::PrimKind::Int),
    ));
    cache.set_cursor(10);
    cache.record(binding, ty, true);
    cache.set_cursor(11);
    assert_eq!(cache.local_type("value"), Some(ty));
}

#[test]
fn contextual_facts_belong_to_callback_bindings_not_the_callers_execution() {
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let first = graph.add_scope(Some(root), 20, 40, true);
    let second = graph.add_scope(Some(root), 50, 80, true);
    let name = graph.intern("value");
    let outer = graph.declare(root, name, 1, Some("Outer".into()));
    let a = graph.declare(first, name, 22, None);
    let b = graph.declare(second, name, 52, Some("Explicit".into()));
    graph.preserve_non_union_type = true;
    let cache = LexicalCache::new(&graph, &arena);
    let ty = arena.intern(Type::Optional(arena.class("Alpha")));
    cache.set_cursor(10);
    cache.record_contextual(a, ty);
    cache.record_contextual(b, arena.class("Beta"));
    assert_eq!(cache.binding("value"), Some(outer));
    assert_eq!(cache.local_type("value"), Some(arena.class("Outer")));
    cache.set_cursor(30);
    assert_eq!(cache.local_type("value"), Some(ty));
    cache.set_cursor(60);
    assert_eq!(cache.local_type("value"), Some(arena.class("Explicit")));
    cache.clear();
    cache.set_cursor(30);
    assert_eq!(cache.local_type("value"), None);
}

#[test]
fn conflicting_contexts_abstain_instead_of_picking_the_latest_type() {
    let arena = TypeArena::new();
    let mut graph = LexicalBindings::default();
    let root = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("value");
    let binding = graph.declare(root, name, 1, None);
    let cache = LexicalCache::new(&graph, &arena);
    cache.record_contextual(binding, arena.class("Alpha"));
    cache.record_contextual(binding, arena.class("Beta"));
    cache.record_contextual(binding, arena.class("Alpha"));
    cache.set_cursor(10);
    assert_eq!(cache.local_type("value"), Some(arena.intern(Type::Unknown)));
}
