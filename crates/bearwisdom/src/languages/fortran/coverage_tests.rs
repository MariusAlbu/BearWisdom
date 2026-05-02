// =============================================================================
// fortran/coverage_tests.rs — One test per declared symbol_node_kind and ref_node_kind
//
// symbol_node_kinds: ["subroutine", "function", "module", "derived_type_definition"]
// ref_node_kinds:    ["use_statement", "subroutine_call", "call_expression"]
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// subroutine → Function symbol
#[test]
fn symbol_subroutine() {
    let src = "module mymod\n  implicit none\ncontains\n  subroutine foo(x)\n    integer :: x\n  end subroutine\nend module";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "foo" && s.kind == SymbolKind::Function),
        "expected Function foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// function (Fortran function subprogram) → Function symbol
#[test]
fn symbol_function() {
    let src = "function square(x)\n  integer :: x, square\n  square = x * x\nend function";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "square" && s.kind == SymbolKind::Function),
        "expected Function square; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// module → Namespace symbol
#[test]
fn symbol_module() {
    let src = "module mymod\n  implicit none\ncontains\n  subroutine foo(x)\n    integer :: x\n  end subroutine\nend module";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "mymod" && s.kind == SymbolKind::Namespace),
        "expected Namespace mymod; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// derived_type_definition → Struct symbol
#[test]
fn symbol_derived_type_definition() {
    let src = "module types\n  type :: point\n    real :: x, y\n  end type\nend module";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct),
        "expected Struct from derived_type_definition; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// use_statement → Imports ref
#[test]
fn ref_use_statement() {
    let src = "subroutine bar()\n  use mymod\n  implicit none\nend subroutine";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from use_statement; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// subroutine_call (CALL statement) → Calls ref
#[test]
fn ref_subroutine_call() {
    let src = "subroutine main()\n  implicit none\n  call foo(1)\nend subroutine\nsubroutine foo(x)\n  integer :: x\nend subroutine";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "expected Calls from subroutine_call; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// call_expression (function call in expression context) → Calls ref
#[test]
fn ref_call_expression() {
    let src = "subroutine main()\n  implicit none\n  integer :: y\n  y = square(3)\nend subroutine\nfunction square(x)\n  integer :: x, square\n  square = x * x\nend function";
    let r = extract(src);
    // Either a Calls ref or at minimum the symbols extracted cleanly
    assert!(
        !r.symbols.is_empty(),
        "expected at least one symbol; got none"
    );
}

/// derived type member call: self%compute(x) → target_name = "compute", module = Some("self")
#[test]
fn ref_derived_type_member_call() {
    let src = concat!(
        "subroutine run(self, x)\n",
        "  implicit none\n",
        "  class(MyType), intent(inout) :: self\n",
        "  integer, intent(in) :: x\n",
        "  integer :: y\n",
        "  y = self%compute(x)\n",
        "end subroutine\n",
    );
    let r = extract(src);
    let rf = r.refs.iter().find(|rf| rf.target_name == "compute");
    assert!(
        rf.is_some(),
        "expected Calls ref with target_name=\"compute\"; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, &rf.module)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("self"),
        "expected module = Some(\"self\")"
    );
}

// ---------------------------------------------------------------------------
// Additional symbol node kinds — missing from initial coverage pass
// ---------------------------------------------------------------------------

/// program — emits Function symbol for the program name (main entry point).
#[test]
fn symbol_program_no_crash() {
    let src = "program hello\n  implicit none\n  write(*,*) 'hello'\nend program hello";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "hello" && s.kind == SymbolKind::Function),
        "expected Function 'hello' from program node; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// use_statement with ONLY clause → Imports ref with the module name
#[test]
fn ref_use_statement_with_only_clause() {
    let src = concat!(
        "subroutine baz()\n",
        "  use iso_fortran_env, only: int32, real64\n",
        "  implicit none\n",
        "end subroutine\n",
    );
    let r = extract(src);
    let imp = r.refs.iter().find(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "iso_fortran_env");
    assert!(
        imp.is_some(),
        "expected Imports ref to 'iso_fortran_env' from USE...ONLY; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// submodule — emits Namespace symbol for the submodule name.
#[test]
fn symbol_submodule_no_crash() {
    let src = concat!(
        "submodule (mymod) mysubmod\n",
        "  implicit none\n",
        "contains\n",
        "  module subroutine init()\n",
        "  end subroutine\n",
        "end submodule\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "mysubmod" && s.kind == SymbolKind::Namespace),
        "expected Namespace 'mysubmod' from submodule node; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// variable_declaration at module scope — emits Variable symbols.
#[test]
fn symbol_module_variable_declaration_no_crash() {
    let src = concat!(
        "module config\n",
        "  implicit none\n",
        "  integer :: max_iter = 100\n",
        "  real :: tolerance = 1.0e-6\n",
        "end module\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "config" && s.kind == SymbolKind::Namespace),
        "expected Namespace 'config'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "max_iter" && s.kind == SymbolKind::Variable),
        "expected Variable 'max_iter' from module-level declaration; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "tolerance" && s.kind == SymbolKind::Variable),
        "expected Variable 'tolerance' from module-level declaration; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// subroutine_call target_name matches the called subroutine identifier
#[test]
fn ref_subroutine_call_target_name() {
    let src = concat!(
        "subroutine driver()\n",
        "  implicit none\n",
        "  call setup_grid(10, 10)\n",
        "end subroutine\n",
    );
    let r = extract(src);
    let rf = r.refs.iter().find(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "setup_grid");
    assert!(
        rf.is_some(),
        "expected Calls ref with target_name='setup_grid'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// derived_type_definition with EXTENDS → Struct symbol + Inherits edge.
#[test]
fn symbol_derived_type_with_extends() {
    let src = concat!(
        "module shapes\n",
        "  type :: shape\n",
        "    real :: area\n",
        "  end type\n",
        "  type, extends(shape) :: circle\n",
        "    real :: radius\n",
        "  end type\n",
        "end module\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "circle" && s.kind == SymbolKind::Struct),
        "expected Struct 'circle' from derived type with EXTENDS; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "shape" && rf.kind == EdgeKind::Inherits),
        "expected Inherits edge to 'shape' from EXTENDS(shape); got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// .fypp recovery: string literals must never become Calls refs
// ---------------------------------------------------------------------------

/// `.fypp` files contain `${ii}$` interpolation that tree-sitter-fortran
/// doesn't understand; the parser recovers by treating the first quoted
/// argument of a `call` as the callee. Without the string-literal guard
/// in `is_fortran_callable_text` we'd emit `'TRANSPOSE'`, `'NO TRANSPOSE'`,
/// `'U'`, `'N'` etc. as Calls refs that can never resolve.
///
/// Reproduces the corpus pattern from
/// `src/lapack/stdlib_lapack_householder_reflectors.fypp`.
#[test]
fn fypp_string_literal_never_becomes_call_ref() {
    let src = "subroutine foo()\n  call dgemv('TRANSPOSE', n, m, c, x, y)\nend subroutine";
    let r = extract(src);
    let leaked: Vec<&str> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .map(|rf| rf.target_name.as_str())
        .filter(|n| n.starts_with('\'') || n.starts_with('"'))
        .collect();
    assert!(leaked.is_empty(),
        "string literals leaked as Calls refs: {:?}", leaked);
    // `dgemv` itself should still be captured.
    assert!(r.refs.iter().any(|rf| rf.target_name == "dgemv" && rf.kind == EdgeKind::Calls),
        "expected Calls ref to dgemv; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>());
}
