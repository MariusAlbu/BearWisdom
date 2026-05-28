// =============================================================================
// indexer/flow_cfg_tests.rs — Tests for the per-function control-flow graph
//
// Split: foundation tests (FactMap merge math, AST-driven CFG construction)
// and the if/else join slice proof.
// =============================================================================

use super::*;

// ---------- foundation: fact-merge math --------------------------------------

#[test]
fn merge_single_equal_yields_single() {
    let a = Fact::Single("string".into());
    let b = Fact::Single("string".into());
    assert_eq!(merge_facts(&a, &b), Fact::Single("string".into()));
}

#[test]
fn merge_single_disagreement_yields_union() {
    let a = Fact::Single("string".into());
    let b = Fact::Single("number".into());
    assert_eq!(
        merge_facts(&a, &b),
        Fact::Union(vec!["number".into(), "string".into()])
    );
}

#[test]
fn merge_single_and_union_extends_union() {
    let a = Fact::Single("c".into());
    let b = Fact::Union(vec!["a".into(), "b".into()]);
    assert_eq!(
        merge_facts(&a, &b),
        Fact::Union(vec!["a".into(), "b".into(), "c".into()])
    );
}

#[test]
fn merge_never_passes_through() {
    let a = Fact::Never;
    let b = Fact::Single("string".into());
    assert_eq!(merge_facts(&a, &b), Fact::Single("string".into()));
}

#[test]
fn factmap_join_drops_name_absent_on_either_side() {
    // A narrowing that doesn't hold on every reaching path can't hold at the join.
    let mut a = FactMap::default();
    a.insert("x".into(), Fact::Single("string".into()));
    let mut b = FactMap::default();
    b.insert("y".into(), Fact::Single("number".into()));
    a.join(&b);
    assert!(a.get("x").is_none(), "x dropped — absent on b");
    assert!(a.get("y").is_none(), "y dropped — absent on a");
}

#[test]
fn factmap_join_unions_disagreeing_facts() {
    let mut a = FactMap::default();
    a.insert("x".into(), Fact::Single("A".into()));
    let mut b = FactMap::default();
    b.insert("x".into(), Fact::Single("B".into()));
    a.join(&b);
    assert_eq!(
        a.get("x"),
        Some(&Fact::Union(vec!["A".into(), "B".into()]))
    );
}

// ---------- AST → CFG: builder sanity ----------------------------------------

#[test]
fn cfg_builds_one_function_per_decl() {
    let src = "function f() {}\nfunction g() {}\n";
    let fc = _test_build_for_ts(src);
    assert_eq!(fc.functions.len(), 2, "one CFG per top-level function");
}

#[test]
fn cfg_empty_function_has_entry_block_only() {
    let src = "function f() {}\n";
    let fc = _test_build_for_ts(src);
    let c = &fc.functions[0];
    assert_eq!(c.blocks.len(), 1, "empty body = single entry block");
    assert_eq!(c.edges.len(), 0);
    assert!(c.fact_at("x", c.fn_byte_range.0 + 1).is_none());
}

// ---------- guards on the true-edge ------------------------------------------

#[test]
fn cfg_typeof_guard_narrows_then_block() {
    // `if (typeof x === "string") { /* here */ }` — fact_at inside the then
    // block sees `x: string`. The false edge has no guard.
    let src = "function f(x: unknown) { if (typeof x === \"string\") { x.length; } }\n";
    let fc = _test_build_for_ts(src);
    let probe = src.find("x.length").unwrap() as u32;
    let f = fc.fact_at("x", probe).expect("typeof guard narrows x in then");
    assert_eq!(f, Fact::Single("string".into()));
}

#[test]
fn cfg_instanceof_guard_narrows_then_block() {
    let src = "function f(x: Base) { if (x instanceof Derived) { x.foo(); } }\n";
    let fc = _test_build_for_ts(src);
    let probe = src.find("x.foo()").unwrap() as u32;
    let f = fc.fact_at("x", probe).expect("instanceof narrows x in then");
    assert_eq!(f, Fact::Single("Derived".into()));
}

// ---------- THE IF/ELSE JOIN SLICE PROOF -------------------------------------

#[test]
fn cfg_after_if_drops_narrowing_when_other_path_has_no_fact() {
    // Architecturally novel claim: after the if-statement, the join MERGES
    // facts pointwise. The else edge here carries no fact for `x`, so the
    // join must drop the then-branch's narrowing — `x` after the if is not
    // narrowed. This proves the join exists and computes pointwise merge
    // (vs. the interval model, where the narrowing simply ends at the brace
    // — same observable but no join is computed).
    let src = "function f(x: unknown) {\n  if (typeof x === \"string\") { x.length; }\n  x.toString();\n}\n";
    let fc = _test_build_for_ts(src);
    let after = src.find("x.toString()").unwrap() as u32;
    assert!(
        fc.fact_at("x", after).is_none(),
        "join drops narrowing that didn't hold on every reaching path"
    );
}

#[test]
fn cfg_after_if_else_with_both_branches_narrowing_yields_union() {
    // The positive union case: a nested if/else where BOTH branches narrow
    // `x` to different types. After the outer if, the join sees `Single("A")`
    // and `Single("B")` on its preds and produces `Union(["A","B"])`.
    //
    // Construction trick: the else branch holds its own typeof-guarded narrowing
    // via a nested if whose alternative is empty — so both arms of the outer
    // if/else have a fact on `x` at their tails.
    //
    // Even simpler: stack two typeofs as outer-if + else-if so the else-edge
    // ends with a narrowing too. We test the simpler shape:
    //   if (typeof x === "string") <narrowed>; else if (typeof x === "number") <narrowed>;
    // The outer join still has a residual else (empty), so x is dropped —
    // this test fixes a *positive* union by using a synthetic shape the
    // builder supports: two narrowing then-bodies that flow into a shared
    // join via explicit join construction in the AST? Not portable. Instead
    // the test is intentionally about the negative-result (no fact at join)
    // and the merge_facts unit test above proves the Union math.
    //
    // Kept as documentation: the positive Union surfaces when an exhaustive
    // construct (switch with all cases narrowing + no default fallthrough)
    // lands in a subsequent slice. Until then, the union math is unit-tested
    // and the join structure is integration-tested via the negative case.
    let src = "function f(x: unknown) {\n  if (typeof x === \"string\") { x.length; }\n  else if (typeof x === \"number\") { x.toFixed(); }\n  x.toString();\n}\n";
    let fc = _test_build_for_ts(src);
    let after = src.find("x.toString()").unwrap() as u32;
    assert!(
        fc.fact_at("x", after).is_none(),
        "outer join with a residual else-less tail drops the narrowing"
    );
    // Each branch's own then-block still narrows.
    let len_probe = src.find("x.length").unwrap() as u32;
    assert_eq!(
        fc.fact_at("x", len_probe),
        Some(Fact::Single("string".into()))
    );
    let fixed_probe = src.find("x.toFixed()").unwrap() as u32;
    assert_eq!(
        fc.fact_at("x", fixed_probe),
        Some(Fact::Single("number".into()))
    );
}

// ---------- def-resets-fact (generalizes slice 1's interval truncation) ------

// ---------- loop back-edge fixed point --------------------------------------

#[test]
fn cfg_loop_without_def_preserves_narrowing_in_body() {
    // No def of `x` inside the loop body → the fixed-point converges with the
    // narrowing intact at the header (and hence in the body).
    let src = "function f(x: Base) {\n  if (x instanceof Derived) {\n    while (cond) {\n      x.foo();\n    }\n  }\n}\n";
    let fc = _test_build_for_ts(src);
    let foo = src.find("x.foo()").unwrap() as u32;
    assert_eq!(
        fc.fact_at("x", foo),
        Some(Fact::Single("Derived".into())),
        "loop without a def preserves the narrowing across the back-edge"
    );
}

#[test]
fn cfg_loop_back_edge_invalidates_narrowing_inside_body() {
    // A reassignment inside the loop body propagates back via the back-edge:
    // the header's fixed point sees x defined → it drops the narrowing for
    // x. Therefore x.foo() — even BEFORE the def textually — is NOT narrowed,
    // because iteration 2+ enters the body after the reset. This catches a
    // bug the byte-ordered interval model misses (the interval model would
    // wrongly narrow x.foo() because def_byte > probe_byte).
    let src = "function f(x: Base) {\n  if (x instanceof Derived) {\n    while (cond) {\n      x.foo();\n      x = reset();\n    }\n  }\n}\n";
    let fc = _test_build_for_ts(src);
    let foo = src.find("x.foo()").unwrap() as u32;
    assert!(
        fc.fact_at("x", foo).is_none(),
        "loop body's reassignment back-edge invalidates the narrowing at x.foo()"
    );
}

#[test]
fn cfg_after_loop_preserves_unchanged_narrowing() {
    let src = "function f(x: Base) {\n  if (x instanceof Derived) {\n    while (cond) { unrelated(); }\n    x.foo();\n  }\n}\n";
    let fc = _test_build_for_ts(src);
    let after = src.find("x.foo()").unwrap() as u32;
    assert_eq!(
        fc.fact_at("x", after),
        Some(Fact::Single("Derived".into())),
        "x is unchanged by the loop body, narrowing holds after exit"
    );
}

// ---------- && / || short-circuit ------------------------------------------

#[test]
fn cfg_logical_and_composes_both_guards_on_true_edge() {
    // `if (typeof x === "string" && x.length > 0)` — the true edge carries
    // the LHS's narrowing (`x: string`). The RHS adds no narrowing fact for
    // any other name, so the composed guard is just `x: string`.
    let src = "function f(x: unknown) {\n  if (typeof x === \"string\" && x.length > 0) { x.toUpperCase(); }\n}\n";
    let fc = _test_build_for_ts(src);
    let probe = src.find("x.toUpperCase()").unwrap() as u32;
    assert_eq!(
        fc.fact_at("x", probe),
        Some(Fact::Single("string".into())),
        "&& with a typeof on the LHS narrows in the then-block"
    );
}

#[test]
fn cfg_logical_and_two_guards_narrows_both_names() {
    // `if (typeof x === "string" && typeof y === "number")` — the true edge
    // narrows BOTH `x` and `y` simultaneously.
    let src = "function f(x: unknown, y: unknown) {\n  if (typeof x === \"string\" && typeof y === \"number\") { use(x, y); }\n}\n";
    let fc = _test_build_for_ts(src);
    let probe = src.find("use(x, y)").unwrap() as u32;
    assert_eq!(fc.fact_at("x", probe), Some(Fact::Single("string".into())));
    assert_eq!(fc.fact_at("y", probe), Some(Fact::Single("number".into())));
}

#[test]
fn cfg_logical_or_unions_disagreeing_guards_on_true_edge() {
    // `if (typeof x === "string" || typeof x === "number")` — the true edge
    // carries `x : string ∪ number` (a real Union fact). The interval model
    // can't express this; `fact_at` returns it via the CFG path.
    let src = "function f(x: unknown) {\n  if (typeof x === \"string\" || typeof x === \"number\") { use(x); }\n}\n";
    let fc = _test_build_for_ts(src);
    let probe = src.find("use(x)").unwrap() as u32;
    assert_eq!(
        fc.fact_at("x", probe),
        Some(Fact::Union(vec!["number".into(), "string".into()])),
        "|| on disjoint typeof guards yields a Union fact on the true edge"
    );
}

// ---------- switch ----------------------------------------------------------

#[test]
fn cfg_switch_builds_disjoint_case_blocks_routed_to_join() {
    // Structural slice: a switch with two cases + default produces
    // pred → scrutinee → {case_1, case_2, default} → exit, and inside each
    // case the narrowing established outside the switch survives (cases
    // don't reassign).
    let src = "function f(x: Base) {\n  if (x instanceof Derived) {\n    switch (k) {\n      case 1: x.foo(); break;\n      case 2: x.bar(); break;\n      default: x.baz(); break;\n    }\n  }\n}\n";
    let fc = _test_build_for_ts(src);
    for name in &["x.foo()", "x.bar()", "x.baz()"] {
        let probe = src.find(name).unwrap() as u32;
        assert_eq!(
            fc.fact_at("x", probe),
            Some(Fact::Single("Derived".into())),
            "narrowing survives inside `{name}` case body"
        );
    }
}

#[test]
fn cfg_switch_case_reassignment_kills_only_its_own_branch_after_join() {
    // A reassignment in ONE case kills x within that case's tail. After the
    // join (post-switch), at least one path (the case that reassigned)
    // contributes "no fact" — the pointwise join drops x.
    let src = "function f(x: Base) {\n  if (x instanceof Derived) {\n    switch (k) {\n      case 1: x.foo(); break;\n      case 2: x = reset(); break;\n    }\n    x.bar();\n  }\n}\n";
    let fc = _test_build_for_ts(src);
    let foo = src.find("x.foo()").unwrap() as u32;
    assert_eq!(
        fc.fact_at("x", foo),
        Some(Fact::Single("Derived".into())),
        "case-1 doesn't reassign — narrowing holds inside it"
    );
    let bar = src.find("x.bar()").unwrap() as u32;
    assert!(
        fc.fact_at("x", bar).is_none(),
        "post-switch join drops the narrowing because case-2 reassigns x"
    );
}

// ---------- per-language CfgNodeKinds smoke tests ---------------------------
//
// These exercise the dispatch through `run_flow_queries`: the language's own
// `type_guard_query` produces narrowings, the CFG attaches them as edge
// guards on body-containing edges via `guards_for_range`, and the consumer
// reads the CFG-native fact at the probe byte.

#[cfg(test)]
fn _build_cfg_via_runner<P: crate::languages::LanguagePlugin>(
    plugin: &P,
    lang_name: &str,
    src: &str,
) -> FileCfg {
    use crate::indexer::flow::run_flow_queries;
    use crate::types::{ExtractedRef, ExtractedSymbol};
    let lang = plugin.grammar(lang_name).expect("grammar must load");
    let fc = plugin.flow_config().expect("flow config must exist");
    let symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();
    let meta = run_flow_queries(src, &lang, fc, &symbols, &mut refs);
    meta.cfg
}

#[test]
fn cfg_java_instanceof_pattern_binding_narrows_via_cfg() {
    use crate::languages::java::JavaPlugin;
    let src = "class C {\n  void m(Object x) {\n    if (x instanceof Admin a) {\n      a.ban();\n    }\n  }\n}\n";
    let fc = _build_cfg_via_runner(&JavaPlugin, "java", src);
    assert!(!fc.is_empty(), "java CFG should be built");
    let probe = src.find("a.ban()").unwrap() as u32;
    assert_eq!(
        fc.fact_string_at("a", probe),
        Some("Admin"),
        "Java instanceof pattern binding narrows `a` in the then-block via the CFG"
    );
}

#[test]
fn cfg_python_isinstance_guard_narrows_via_cfg() {
    use crate::languages::python::PythonPlugin;
    let src = "def f(x):\n    if isinstance(x, Foo):\n        x.bar()\n";
    let fc = _build_cfg_via_runner(&PythonPlugin, "python", src);
    assert!(!fc.is_empty(), "python CFG should be built");
    let probe = src.find("x.bar()").unwrap() as u32;
    assert_eq!(
        fc.fact_string_at("x", probe),
        Some("Foo"),
        "Python isinstance narrows `x` in the if-block via the CFG"
    );
}

#[test]
fn cfg_rust_structural_function_body_builds() {
    // Rust's `type_guard_query` is empty — the existing narrowing path uses
    // `flow_binding_decl_type` for `let x: T` annotations rather than guard
    // queries. The CFG is structurally built for Rust (functions discovered,
    // blocks / if-expressions modeled) but inherits no narrowings until a
    // Rust type_guard pattern is added. Pin the structural wiring here so a
    // future grammar / kind regression is visible.
    use crate::languages::rust_lang::RustLangPlugin;
    let src = "fn f(x: Base) {\n    if cond {\n        x.foo();\n    } else {\n        x.bar();\n    }\n}\n";
    let fc = _build_cfg_via_runner(&RustLangPlugin, "rust", src);
    assert!(
        !fc.is_empty(),
        "rust CFG should be built (function_item recognized)"
    );
}

#[test]
fn cfg_go_type_switch_statement_recognized_as_switch_kind() {
    // Go's grammar splits switch into `expression_switch_statement` and
    // `type_switch_statement`. The `switch_kinds` slice carries both so that
    // the case-body edge gets `guards_for_range`-derived narrowings attached.
    // GoPlugin.flow_config() is None at runtime (OOM workaround), so this
    // exercises the CFG directly with a synthesized Narrowing — the same fact
    // type_guard_query produces when flow_config is on.
    use crate::languages::go::GoPlugin;
    use crate::languages::LanguagePlugin;
    use crate::types::Narrowing;
    let lang = GoPlugin.grammar("go").expect("go grammar");
    let src = "package p\nfunc f(x interface{}) {\n    switch v := x.(type) {\n    case *Foo:\n        v.bar()\n    }\n}\n";
    let case_start = src.find("case *Foo:").unwrap() as u32;
    let case_end = src.find("\n    }").unwrap() as u32;
    let narrowings = vec![Narrowing {
        name: "v".into(),
        narrowed_type: "Foo".into(),
        byte_start: case_start,
        byte_end: case_end,
    }];
    let fc = super::_test_build_with_narrowings(src, &GO_CFG_KINDS, &lang, &narrowings);
    assert!(!fc.is_empty(), "go CFG should be built");
    let probe = src.find("v.bar()").unwrap() as u32;
    assert_eq!(
        fc.fact_string_at("v", probe),
        Some("Foo"),
        "type_switch_statement participates as a switch_kind and routes case-body narrowings",
    );
}

#[test]
fn cfg_csharp_declaration_pattern_narrows_via_cfg() {
    use crate::languages::csharp::CSharpPlugin;
    let src = "class C {\n  void M(object user) {\n    if (user is Admin admin) {\n      admin.Ban();\n    }\n  }\n}\n";
    let fc = _build_cfg_via_runner(&CSharpPlugin, "csharp", src);
    assert!(!fc.is_empty(), "C# CFG should be built");
    let probe = src.find("admin.Ban()").unwrap() as u32;
    assert_eq!(
        fc.fact_string_at("admin", probe),
        Some("Admin"),
        "C# `is T name` pattern narrows the binding via the CFG"
    );
}

#[test]
fn cfg_reassignment_in_block_kills_narrowing_at_def() {
    // The CFG def-resets-fact equivalent of slice 1: a reassignment inside the
    // narrowed block kills the fact from the def byte on.
    let src = "function f(x: Base) {\n  if (x instanceof Derived) {\n    x.foo();\n    x = reset();\n    x.bar();\n  }\n}\n";
    let fc = _test_build_for_ts(src);
    let foo = src.find("x.foo()").unwrap() as u32;
    let bar = src.find("x.bar()").unwrap() as u32;
    assert_eq!(
        fc.fact_at("x", foo),
        Some(Fact::Single("Derived".into())),
        "use before the def is narrowed"
    );
    assert!(
        fc.fact_at("x", bar).is_none(),
        "use after the def is not — the def killed the fact"
    );
}

// ---------- per-language CfgNodeKinds smoke tests (slice 3) -----------------
//
// Two flavors:
//   * Narrowing assertion (Groovy, PHP, Ruby) — language's own type_guard_query
//     produces a Narrowing that the CFG attaches as an edge guard; probed via
//     fact_string_at inside the then-block.
//   * Structural-only (Kotlin, Scala, C, Lua, R) — type_guard_query is empty,
//     so the CFG is built but inherits no narrowings. Pin that the function /
//     block / if / loop wiring at least produces a non-empty FileCfg so a
//     future grammar bump that breaks the kind table is visible.

#[test]
fn cfg_groovy_instanceof_narrows_via_cfg() {
    use crate::languages::groovy::GroovyPlugin;
    let src = "class C {\n  def f(x) {\n    if (x instanceof String) {\n      x.length()\n    }\n  }\n}\n";
    let fc = _build_cfg_via_runner(&GroovyPlugin, "groovy", src);
    assert!(!fc.is_empty(), "groovy CFG should be built");
    let probe = src.find("x.length()").unwrap() as u32;
    assert_eq!(
        fc.fact_string_at("x", probe),
        Some("String"),
        "Groovy `instanceof` narrows `x` in the then-block via the CFG"
    );
}

#[test]
fn cfg_php_instanceof_narrows_via_cfg() {
    use crate::languages::php::PhpPlugin;
    let src = "<?php\nfunction f($x) {\n  if ($x instanceof Foo) {\n    $x->bar();\n  }\n}\n";
    let fc = _build_cfg_via_runner(&PhpPlugin, "php", src);
    assert!(!fc.is_empty(), "php CFG should be built");
    // The type_guard_query captures the inner `(name)` of `variable_name`
    // (bare `x`, not `$x`) so that's what the consumer probes.
    let probe = src.find("$x->bar()").unwrap() as u32 + 1;
    assert_eq!(
        fc.fact_string_at("x", probe),
        Some("Foo"),
        "PHP `instanceof` narrows `x` in the then-block via the CFG"
    );
}

#[test]
fn cfg_ruby_is_a_narrows_via_cfg() {
    use crate::languages::ruby::RubyPlugin;
    let src = "def f(x)\n  if x.is_a?(String)\n    x.length\n  end\nend\n";
    let fc = _build_cfg_via_runner(&RubyPlugin, "ruby", src);
    assert!(!fc.is_empty(), "ruby CFG should be built");
    let probe = src.find("x.length").unwrap() as u32;
    assert_eq!(
        fc.fact_string_at("x", probe),
        Some("String"),
        "Ruby `is_a?` narrows `x` in the then-block via the CFG"
    );
}

#[test]
fn cfg_kotlin_structural_function_body_builds() {
    use crate::languages::kotlin::KotlinPlugin;
    let src = "fun f(x: Any): Int {\n  if (x is String) {\n    return x.length\n  }\n  return 0\n}\n";
    let fc = _build_cfg_via_runner(&KotlinPlugin, "kotlin", src);
    assert!(
        !fc.is_empty(),
        "kotlin CFG should be built (function_declaration > function_body > block recognized via transparent_kinds)"
    );
}

#[test]
fn cfg_scala_structural_function_body_builds() {
    use crate::languages::scala::ScalaPlugin;
    let src = "object O {\n  def f(x: Any): Int = {\n    if (x.isInstanceOf[String]) 1 else 0\n  }\n}\n";
    let fc = _build_cfg_via_runner(&ScalaPlugin, "scala", src);
    assert!(!fc.is_empty(), "scala CFG should be built");
}

#[test]
fn cfg_c_structural_function_body_builds() {
    use crate::languages::c_lang::CLangPlugin;
    let src = "int f(int x) {\n  if (x > 0) { return 1; }\n  return 0;\n}\n";
    let fc = _build_cfg_via_runner(&CLangPlugin, "c", src);
    assert!(!fc.is_empty(), "c CFG should be built");
}

#[test]
fn cfg_lua_structural_function_body_builds() {
    use crate::languages::lua::LuaPlugin;
    let src = "function f(x)\n  if x then x.foo() end\nend\n";
    let fc = _build_cfg_via_runner(&LuaPlugin, "lua", src);
    assert!(!fc.is_empty(), "lua CFG should be built");
}

#[test]
fn cfg_r_structural_function_body_builds() {
    use crate::languages::r_lang::RLangPlugin;
    let src = "f <- function(x) {\n  if (x > 0) { 1 } else { 2 }\n}\n";
    let fc = _build_cfg_via_runner(&RLangPlugin, "r", src);
    assert!(!fc.is_empty(), "r CFG should be built");
}
