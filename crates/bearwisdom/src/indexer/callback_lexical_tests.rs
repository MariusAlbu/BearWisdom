use super::*;

fn capture_graph(language: &str, source: &str) -> (Option<LexicalBindings>, Vec<ExtractedRef>) {
    let refs = match language {
        "scala" => crate::languages::scala::extract::extract(source).refs,
        "java" => crate::languages::java::extract::extract(source).refs,
        "csharp" => crate::languages::csharp::extract::extract(source).refs,
        "kotlin" => crate::languages::kotlin::extract::extract(source).refs,
        "swift" => crate::languages::swift::extract::extract(source).refs,
        "dart" => crate::languages::dart::extract::extract(source).refs,
        "go" => crate::languages::go::extract::extract(source).refs,
        "python" => crate::languages::python::extract::extract(source).refs,
        _ => unreachable!(),
    };
    let plugin = crate::languages::default_registry().get(language);
    let grammar = plugin.grammar(language).expect("grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(source, None).unwrap();
    (
        capture(tree.root_node(), source.as_bytes(), language, &refs),
        refs,
    )
}

fn capture_source(language: &str, source: &str) -> (LexicalBindings, Vec<ExtractedRef>) {
    let (graph, refs) = capture_graph(language, source);
    (graph.expect("callback graph"), refs)
}

fn root_at(refs: &[ExtractedRef], source: &str, needle: &str) -> u32 {
    let byte = source.find(needle).expect("source marker") as u32;
    refs.iter()
        .find(|reference| reference.byte_offset == byte)
        .map(|reference| reference.byte_offset)
        .unwrap_or_else(|| panic!("missing root ref at {needle:?}: {refs:?}"))
}

fn reference_for<'a>(refs: &'a [ExtractedRef], root: &str, target: &str) -> &'a ExtractedRef {
    refs.iter()
        .find(|reference| {
            reference.target_name == target
                && reference
                    .chain
                    .as_ref()
                    .and_then(|chain| chain.segments.first())
                    .is_some_and(|segment| segment.name == root)
        })
        .unwrap_or_else(|| panic!("missing {root}.{target} ref: {refs:?}"))
}

fn declaration_at(
    graph: &LexicalBindings,
    source: &str,
    marker: &str,
) -> crate::indexer::lexical::BindingId {
    let start = source.find(marker).expect("parameter marker") as u32;
    graph.declarations[&SourceSpan {
        start,
        end: start + 1,
    }]
}

#[test]
fn scala_nested_same_name_callback_roots_use_innermost_binding() {
    let source = "object O { def f(xs: List[A], ys: List[B]) = xs.map(x => { ys.map(x => x.inner()); x.outer() }) }";
    let (graph, refs) = capture_source("scala", source);
    let inner = root_at(&refs, source, "x.inner");
    let outer = root_at(&refs, source, "x.outer");
    let inner_binding = graph.references[&inner];
    let outer_binding = graph.references[&outer];
    assert_ne!(
        inner_binding, outer_binding,
        "nested names must not share a binding"
    );
    assert_eq!(
        graph.binding_at(inner, graph.name_id("x").unwrap()),
        Some(inner_binding)
    );
    assert_eq!(
        graph.binding_at(outer, graph.name_id("x").unwrap()),
        Some(outer_binding)
    );
}

#[test]
fn scala_sibling_and_outside_same_name_roots_do_not_leak() {
    let source = "object O { def f(xs: List[A], ys: List[B], x: C) = { xs.map(x => x.left()); ys.map(x => x.right()); x.outside() } }";
    let (graph, refs) = capture_source("scala", source);
    let left = root_at(&refs, source, "x.left");
    let right = root_at(&refs, source, "x.right");
    let outside = root_at(&refs, source, "x.outside");
    assert_ne!(graph.references[&left], graph.references[&right]);
    assert!(
        !graph.references.contains_key(&outside),
        "ordinary method parameter must not become a callback binding"
    );
}

#[test]
fn scala_local_shadow_barrier_abstains_after_the_declaration() {
    let source =
        "object O { def f(xs: List[A], other: B) = xs.map(x => { val x = other; x.shadow() }) }";
    let (graph, refs) = capture_source("scala", source);
    let shadow = root_at(&refs, source, "x.shadow");
    assert!(
        !graph.references.contains_key(&shadow),
        "a local declaration must block attribution to the callback parameter"
    );
}

#[test]
fn outer_callback_capture_survives_a_nested_callback_without_that_name() {
    for (language, source, marker, target) in [
        (
            "scala",
            "object O { def f(xs: List[A], ys: List[B]) = xs.map(x => ys.map(y => x.outer())) }",
            "x =>",
            "outer",
        ),
        (
            "java",
            "class O { void f(java.util.List<A> xs, java.util.List<B> ys) { xs.forEach(x -> ys.forEach(y -> x.outer())); } }",
            "x ->",
            "outer",
        ),
        (
            "csharp",
            "class O { void F(System.Collections.Generic.List<A> xs, System.Collections.Generic.List<B> ys) { xs.ForEach(x => ys.ForEach(y => x.Outer())); } }",
            "x =>",
            "Outer",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let reference = reference_for(&refs, "x", target);
        assert_eq!(
            graph.references.get(&reference.byte_offset),
            Some(&declaration_at(&graph, source, marker)),
            "{language}: an inner callback without x must retain the outer x capture"
        );
    }
}

#[test]
fn ordinary_boundary_inside_differently_named_nested_callback_fences_outer_capture() {
    let source = "object O { def f(xs: List[A], ys: List[B]) = xs.map(x => ys.map(y => { def nested(x: C): Unit = x.inner(); x.outer() })) }";
    let (graph, refs) = capture_source("scala", source);
    let inner = reference_for(&refs, "x", "inner");
    let outer = reference_for(&refs, "x", "outer");
    assert!(
        !graph.references.contains_key(&inner.byte_offset),
        "the nested function parameter must not bind to the outer callback"
    );
    assert!(
        graph.references.contains_key(&outer.byte_offset),
        "the differently named nested callback must preserve the outer capture"
    );
}

#[test]
fn java_selector_anchored_ref_still_attests_its_receiver_parameter() {
    let source = "class O { void f(java.util.List<A> xs) { xs.forEach(x -> x.foo()); } }";
    let (graph, refs) = capture_source("java", source);
    let reference = reference_for(&refs, "x", "foo");
    assert_ne!(
        reference.byte_offset as usize,
        source.find("x.foo").unwrap(),
        "Java deliberately addresses method refs at the selector"
    );
    assert_eq!(
        graph.references.get(&reference.byte_offset),
        Some(&declaration_at(&graph, source, "x ->"))
    );
}

#[test]
fn java_and_csharp_declaration_spans_are_exact() {
    for (language, source, parameter) in [
        (
            "java",
            "class O { void f() { use((Thing x) -> x.touch()); } }",
            "x) ->",
        ),
        (
            "csharp",
            "class O { void F() { Use((Thing x) => x.Touch()); } }",
            "x) =>",
        ),
    ] {
        let (graph, _) = capture_source(language, source);
        let start = source.find(parameter).unwrap() as u32;
        let declaration = SourceSpan {
            start,
            end: start + 1,
        };
        assert!(
            graph.declarations.contains_key(&declaration),
            "{language}: {graph:?}"
        );
        assert!(
            !graph.declarations.contains_key(&SourceSpan {
                start,
                end: start + 2,
            }),
            "{language}: partial/extended source ranges are not declaration identity"
        );
    }
}

#[test]
fn nested_ordinary_callable_bodies_do_not_capture_outer_callback_parameters() {
    for (language, source, blocked, preserved) in [
        (
            "scala",
            "object O { def f(xs: List[A]) = xs.map(x => { def nested(x: B): Unit = x.local(); x.outer() }) }",
            "local",
            "outer",
        ),
        (
            "csharp",
            "class O { void F(System.Collections.Generic.List<A> xs) { xs.ForEach(x => { void Nested(B x) { x.Local(); } x.Outer(); }); } }",
            "Local",
            "Outer",
        ),
        (
            "java",
            "class O { void f(java.util.List<A> xs) { xs.forEach(x -> { class Local { void run() { x.local(); } } new Runnable() { public void run() { x.anonymous(); } }.run(); x.outer(); }); } }",
            "local",
            "outer",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let blocked = reference_for(&refs, "x", blocked);
        let preserved = reference_for(&refs, "x", preserved);
        assert!(
            !graph.references.contains_key(&blocked.byte_offset),
            "{language}: refs inside an ordinary nested boundary must abstain"
        );
        assert!(
            graph.references.contains_key(&preserved.byte_offset),
            "{language}: direct callback-body reads must remain eligible"
        );
    }
}

#[test]
fn java_anonymous_class_method_body_does_not_capture_outer_callback_parameter() {
    let source = "class O { void f(java.util.List<A> xs) { xs.forEach(x -> new Object() { void run(B x) { x.anonymous(); } }); } }";
    let (graph, refs) = capture_source("java", source);
    let reference = reference_for(&refs, "x", "anonymous");
    assert!(
        !graph.references.contains_key(&reference.byte_offset),
        "anonymous class method bodies must fence the outer callback parameter"
    );
}

#[test]
fn kotlin_and_swift_explicit_callback_parameter_spans_and_refs_are_exact() {
    for (language, source, marker, target) in [
        (
            "kotlin",
            "fun f(xs: List<A>) { xs.map { x -> x.touch() } }",
            "x ->",
            "touch",
        ),
        (
            "swift",
            "func f(xs: [A]) { xs.map { x in x.touch() } }",
            "x in",
            "touch",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let reference = reference_for(&refs, "x", target);
        assert_eq!(
            graph.references.get(&reference.byte_offset),
            Some(&declaration_at(&graph, source, marker)),
            "{language}: callback ref must attest the exact explicit declaration"
        );
    }
}

#[test]
fn kotlin_and_swift_nested_callback_identity_and_outer_capture_are_scoped() {
    for (language, source, inner_target, outer_target) in [
        (
            "kotlin",
            "fun f(xs: List<A>, ys: List<B>) { xs.map { x -> ys.map { x -> x.inner() }; x.outer() } }",
            "inner",
            "outer",
        ),
        (
            "swift",
            "func f(xs: [A], ys: [B]) { xs.map { x in ys.map { x in x.inner() }; x.outer() } }",
            "inner",
            "outer",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let inner = reference_for(&refs, "x", inner_target);
        let outer = reference_for(&refs, "x", outer_target);
        assert_ne!(
            graph.references[&inner.byte_offset], graph.references[&outer.byte_offset],
            "{language}: nested same-name declarations must remain distinct"
        );
    }

    for (language, source, target) in [
        (
            "kotlin",
            "fun f(xs: List<A>, ys: List<B>) { xs.map { x -> ys.map { y -> x.outer() } } }",
            "outer",
        ),
        (
            "swift",
            "func f(xs: [A], ys: [B]) { xs.map { x in ys.map { y in x.outer() } } }",
            "outer",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let reference = reference_for(&refs, "x", target);
        assert!(
            graph.references.contains_key(&reference.byte_offset),
            "{language}: a differently named nested callback must preserve outer capture"
        );
    }
}

#[test]
fn kotlin_and_swift_local_shadow_and_nested_callable_boundaries_abstain() {
    for (language, source, blocked, preserved) in [
        (
            "kotlin",
            "fun f(xs: List<A>, other: B) { xs.map { x -> if (true) { val x = other; x.shadow() }; x.outer() } }",
            "shadow",
            "outer",
        ),
        (
            "swift",
            "func f(xs: [A], other: B) { xs.map { x in if true { let x = other; x.shadow() }; x.outer() } }",
            "shadow",
            "outer",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let blocked = reference_for(&refs, "x", blocked);
        let preserved = reference_for(&refs, "x", preserved);
        assert!(
            !graph.references.contains_key(&blocked.byte_offset),
            "{language}: nested local shadow must abstain"
        );
        assert!(
            graph.references.contains_key(&preserved.byte_offset),
            "{language}: callback read outside the local scope must remain eligible"
        );
    }

    for (language, source, blocked, preserved) in [
        (
            "kotlin",
            "fun f(xs: List<A>) { xs.map { x -> fun nested(x: B) { x.inner() }; x.outer() } }",
            "inner",
            "outer",
        ),
        (
            "swift",
            "func f(xs: [A]) { xs.map { x in func nested(x: B) { x.inner() }; x.outer() } }",
            "inner",
            "outer",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let blocked = reference_for(&refs, "x", blocked);
        let preserved = reference_for(&refs, "x", preserved);
        assert!(
            !graph.references.contains_key(&blocked.byte_offset),
            "{language}: nested callable body must abstain"
        );
        assert!(
            graph.references.contains_key(&preserved.byte_offset),
            "{language}: direct callback read must remain eligible"
        );
    }
}

#[test]
fn kotlin_it_and_swift_shorthand_closures_do_not_invent_declaration_identity() {
    for (language, source) in [
        ("kotlin", "fun f(xs: List<A>) { xs.map { it.touch() } }"),
        ("swift", "func f(xs: [A]) { xs.map { $0.touch() } }"),
    ] {
        let (graph, _) = capture_graph(language, source);
        assert!(
            graph.is_none(),
            "{language}: implicit callback parameters have no declaration span"
        );
    }
}

#[test]
fn kotlin_and_swift_reassignment_fences_later_callback_reads() {
    for (language, source, before, after) in [
        (
            "kotlin",
            "fun f(xs: List<A>, other: A) { xs.map { x -> x.before(); x = other; x.after() } }",
            "before",
            "after",
        ),
        (
            "swift",
            "func f(xs: [A], other: A) { xs.map { x in x.before(); x = other; x.after() } }",
            "before",
            "after",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let before = reference_for(&refs, "x", before);
        let after = reference_for(&refs, "x", after);
        assert!(
            graph.references.contains_key(&before.byte_offset),
            "{language}: reads before the write retain callback identity"
        );
        assert!(
            !graph.references.contains_key(&after.byte_offset),
            "{language}: reads after reassignment must abstain"
        );
    }
}

#[test]
fn dart_and_go_callback_parameters_are_exact_and_nested_scoped() {
    for (language, source, declaration, inner_target, outer_target) in [
        (
            "dart",
            "void f(List<A> xs, List<B> ys) { xs.map((A x) { ys.map((B x) => x.inner()); x.outer(); }); }",
            "x) {",
            "inner",
            "outer",
        ),
        (
            "go",
            "func f(xs []A, ys []B) { xs.Map(func(x A) { ys.Map(func(x B) { x.Inner() }); x.Outer() }) }",
            "x A) {",
            "Inner",
            "Outer",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let inner = reference_for(&refs, "x", inner_target);
        let outer = reference_for(&refs, "x", outer_target);
        assert_ne!(
            graph.references[&inner.byte_offset], graph.references[&outer.byte_offset],
            "{language}: same-name nested callbacks need distinct declaration identity"
        );
        assert_eq!(
            graph.references.get(&outer.byte_offset),
            Some(&declaration_at(&graph, source, declaration)),
            "{language}: the outer reference must attest its exact declaration token"
        );
    }

    for (language, source, target) in [
        (
            "dart",
            "void f(List<A> xs, List<B> ys) { xs.map((A x) { ys.map((B y) => x.outer()); }); }",
            "outer",
        ),
        (
            "go",
            "func f(xs []A, ys []B) { xs.Map(func(x A) { ys.Map(func(y B) { x.Outer() }) }) }",
            "Outer",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let captured = reference_for(&refs, "x", target);
        assert!(
            graph.references.contains_key(&captured.byte_offset),
            "{language}: a differently named nested callback must retain outer capture"
        );
    }
}

#[test]
fn dart_and_go_local_shadow_and_reassignment_fence_callback_identity() {
    for (language, source, shadow, outer, before, after) in [
        (
            "dart",
            "void f(List<A> xs, A other) { xs.map((A x) { { var x = other; x.shadow(); } x.outer(); x.before(); x = other; x.after(); }); }",
            "shadow",
            "outer",
            "before",
            "after",
        ),
        (
            "go",
            "func f(xs []A, other A) { xs.Map(func(x A) { { x := other; x.Shadow() }; x.Outer(); x.Before(); x = other; x.After() }) }",
            "Shadow",
            "Outer",
            "Before",
            "After",
        ),
    ] {
        let (graph, refs) = capture_source(language, source);
        let shadow = reference_for(&refs, "x", shadow);
        let outer = reference_for(&refs, "x", outer);
        let before = reference_for(&refs, "x", before);
        let after = reference_for(&refs, "x", after);
        assert!(
            !graph.references.contains_key(&shadow.byte_offset),
            "{language}: local shadow reads must abstain"
        );
        assert!(
            graph.references.contains_key(&outer.byte_offset),
            "{language}: local-block shadow must not suppress the enclosing callback read"
        );
        assert!(
            graph.references.contains_key(&before.byte_offset),
            "{language}: reads before a write retain the declaration identity"
        );
        assert!(
            !graph.references.contains_key(&after.byte_offset),
            "{language}: reads after reassignment must abstain"
        );
    }
}

#[test]
fn dart_local_callable_boundary_does_not_leak_callback_identity() {
    let source =
        "void f(List<A> xs) { xs.map((A x) { void nested(B x) { x.local(); } x.outer(); }); }";
    let (graph, refs) = capture_source("dart", source);
    let nested = reference_for(&refs, "x", "local");
    let outer = reference_for(&refs, "x", "outer");
    assert!(
        !graph.references.contains_key(&nested.byte_offset),
        "a local callable body must not borrow its enclosing callback parameter identity"
    );
    assert!(
        graph.references.contains_key(&outer.byte_offset),
        "the enclosing callback body remains eligible after its nested callable"
    );
}

#[test]
fn python_direct_lambda_parameters_are_exact_including_underscore() {
    for (source, marker, target) in [
        (
            "def f(xs):\n    return xs.map(lambda x: x.touch())\n",
            "x: x.touch",
            "touch",
        ),
        (
            "def f(xs):\n    return xs.map(lambda _: _.touch())\n",
            "_: _.touch",
            "touch",
        ),
    ] {
        let (graph, refs) = capture_source("python", source);
        let name = if marker.starts_with('_') { "_" } else { "x" };
        let reference = reference_for(&refs, name, target);
        assert_eq!(
            graph.references.get(&reference.byte_offset),
            Some(&declaration_at(&graph, source, marker)),
            "Python direct lambda identifiers must attest their exact declaration token"
        );
    }

    for source in [
        "def f(xs, other):\n    return xs.map(lambda x=other: x.touch())\n",
        "def f(xs):\n    return xs.map(lambda *x: x.touch())\n",
        "def f(xs):\n    return xs.map(lambda **x: x.touch())\n",
    ] {
        assert!(
            capture_graph("python", source).0.is_none(),
            "default and splat lambda slots are emitter holes, not declaration identity"
        );
    }
}

#[test]
fn python_nested_lambdas_capture_outer_or_declare_innermost() {
    let source =
        "def f(xs, ys):\n    return xs.map(lambda x: ys.map(lambda x: x.inner()) or x.outer())\n";
    let (graph, refs) = capture_source("python", source);
    let inner = reference_for(&refs, "x", "inner");
    let outer = reference_for(&refs, "x", "outer");
    assert_ne!(
        graph.references[&inner.byte_offset], graph.references[&outer.byte_offset],
        "nested same-name lambda parameters must not share a binding"
    );

    let source = "def f(xs, ys):\n    return xs.map(lambda x: ys.map(lambda y: x.outer()))\n";
    let (graph, refs) = capture_source("python", source);
    let outer = reference_for(&refs, "x", "outer");
    assert!(
        graph.references.contains_key(&outer.byte_offset),
        "a differently named nested lambda must retain outer capture"
    );
}

#[test]
fn python_walrus_and_comprehension_targets_fence_only_their_scope() {
    let source = "def f(xs, other):\n    return xs.map(lambda x: x.before() and (x := other) and x.after())\n";
    let (graph, refs) = capture_source("python", source);
    let before = reference_for(&refs, "x", "before");
    let after = reference_for(&refs, "x", "after");
    assert!(graph.references.contains_key(&before.byte_offset));
    assert!(
        !graph.references.contains_key(&after.byte_offset),
        "a walrus rebind must fence later callback reads"
    );

    let source = "def f(xs, values):\n    return xs.map(lambda x: [x.shadow() for x in values] and x.outer())\n";
    let (graph, refs) = capture_source("python", source);
    let shadow = reference_for(&refs, "x", "shadow");
    let outer = reference_for(&refs, "x", "outer");
    assert!(
        !graph.references.contains_key(&shadow.byte_offset),
        "a comprehension target must shadow the lambda parameter in its own scope"
    );
    assert!(
        graph.references.contains_key(&outer.byte_offset),
        "a comprehension target must not suppress a later outer lambda read"
    );
}

#[test]
fn python_callable_and_class_boundaries_are_explicitly_fenced() {
    // Python lambda bodies are expressions, so a statement-defined function
    // or class cannot be nested syntactically inside one. Assert the boundary
    // contract directly to guard the traversal configuration that applies
    // whenever such a callable/class is encountered below a captured callback
    // through another expression subtree.
    assert!(ordinary_boundary_kind("python", "function_definition"));
    assert!(ordinary_boundary_kind("python", "class_definition"));
    assert!(!ordinary_boundary_kind("python", "lambda"));
}

#[test]
fn python_bare_assignment_is_a_name_only_barrier() {
    let source = "x = other\n";
    let plugin = crate::languages::default_registry().get("python");
    let grammar = plugin.grammar("python").expect("grammar");
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let root = tree.root_node();
    let barriers = collect_barriers(root, "python", span(root));
    assert_eq!(barriers.len(), 1);
    assert_eq!(text(source.as_bytes(), barriers[0].name), Some("x"));
    assert_eq!(barriers[0].range, span(root));
}
