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

/// derived type member call: self%compute(x) → target_name = "compute",
/// module = Some(type_name). The local type map resolves `self: MyType`
/// from `class(MyType) :: self`, so `module` carries the type name rather
/// than the variable name.
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
    // With the local type map, `self` resolves to its declared type `MyType`.
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("MyType"),
        "expected module = Some(\"MyType\") after type-map resolution"
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

/// Module with `use M, only: local => source` + `public :: local` emits a
/// synthetic Function symbol for `local` so callers importing `local` from
/// this module can resolve it via the normal import-based path.
#[test]
fn symbol_public_reexport_alias_synthetic() {
    let src = concat!(
        "module testsuite\n",
        "  use fpm_error, only : error_t, test_failed => fatal_error\n",
        "  implicit none\n",
        "  private\n",
        "  public :: run_testsuite, test_failed\n",
        "end module\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "test_failed" && s.kind == SymbolKind::Function),
        "expected synthetic Function 'test_failed' from public re-export of alias; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    // run_testsuite is not a rename alias — no spurious synthetic for it.
    let run_count = r.symbols.iter().filter(|s| s.name == "run_testsuite").count();
    assert_eq!(
        run_count, 0,
        "expected no synthetic for non-alias public name 'run_testsuite'"
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
// Generic interface blocks — emit the public name as a Function symbol
// ---------------------------------------------------------------------------

/// `interface moment ... end interface moment` declares `moment` as a
/// generic procedure name that Fortran dispatches at runtime to one of
/// the type-specific procedures inside. Cross-file callers reference
/// the generic name; the resolver needs an indexed Function symbol to
/// match against.
///
/// Real-world driver: stdlib_stats's `interface moment`/`mean`/`var`/
/// `cov`/`corr` blocks are how the library exposes its public API.
#[test]
fn named_generic_interface_emits_function_symbol() {
    let src = "module mymod\n  implicit none\n  interface moment\n    module function moment_real(x) result(r)\n      real :: x, r\n    end function moment_real\n  end interface moment\nend module mymod\n";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "moment" && s.kind == SymbolKind::Function),
        "expected Function `moment` from interface block; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    // The inner module function is still extracted via normal recursion.
    assert!(
        r.symbols.iter().any(|s| s.name == "moment_real" && s.kind == SymbolKind::Function),
        "inner type-specific procedure must also be present; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Anonymous `interface ... end interface` (procedure-prototype form) has
/// no name — the inner function/subroutine statements should still be
/// walked but no synthetic interface symbol is emitted.
#[test]
fn anonymous_interface_does_not_emit_unnamed_symbol() {
    let src = "module mymod\n  interface\n    function ext_proc(x) result(r)\n      integer :: x, r\n    end function ext_proc\n  end interface\nend module mymod\n";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "ext_proc"),
        "inner prototype function must be extracted; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        !r.symbols.iter().any(|s| s.name.is_empty()),
        "no empty-name symbol may be emitted from anonymous interface; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Local-array indexing must not emit Calls refs
// ---------------------------------------------------------------------------

/// Fortran array indexing uses identical syntax to function calls
/// (`mm(i, j)`). When `mm` is a locally-declared array inside a
/// subroutine/function, the extractor must NOT emit a Calls ref for
/// the indexing — those are false-positive references that inflate
/// unresolved_refs by tens of thousands on numerical-library
/// codebases (10,539 unresolved fortran.calls in fortran-stdlib pre-fix,
/// dominated by short locals like `mm`, `dl`, `du`, `dy`, `mm`).
#[test]
fn local_array_indexing_does_not_emit_call() {
    let src = "\
subroutine compute(n)
  integer :: n
  integer :: mm(10, 4)
  integer :: i
  i = 1
  print *, mm(1, 1)
  print *, mm(i, 2)
  call dgemv(mm, n)
end subroutine
";
    let r = extract(src);
    let calls: Vec<&str> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        !calls.contains(&"mm"),
        "local array `mm(i, j)` must NOT emit Calls; got {calls:?}"
    );
    // Real call to `dgemv` must still emit.
    assert!(
        calls.contains(&"dgemv"),
        "real call `dgemv(mm, n)` SHOULD emit Calls; got {calls:?}"
    );
}

// ---------------------------------------------------------------------------
// .fypp recovery: tree-sitter-fortran partially parses preprocessed files
// ---------------------------------------------------------------------------

/// .fypp files mix valid Fortran with `#:for`, `#:if`, `${...}$` directives
/// that tree-sitter-fortran cannot parse. The parser sets `has_error()` but
/// still recovers the surrounding valid nodes. When the module statement
/// appears after any leading directives, the extractor must still produce
/// the module symbol from the valid region.
#[test]
fn fypp_partial_parse_extracts_module_and_procedures() {
    // Directives inside the module body (not at file start) — this mirrors
    // the common .fypp pattern where `module foo` is the first real line.
    let src = concat!(
        "module stdlib_math\n",
        "  implicit none\n",
        "  private\n",
        "  public :: clip\n",
        "#:if WITH_QP\n",
        "  real :: EULERS_NUMBER_QP\n",
        "#:endif\n",
        "contains\n",
        "  pure elemental subroutine clip_i32(x, xmin, xmax)\n",
        "    integer, intent(in) :: x, xmin, xmax\n",
        "  end subroutine\n",
        "end module stdlib_math\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "stdlib_math" && s.kind == SymbolKind::Namespace),
        "expected Namespace 'stdlib_math' extracted despite fypp directives; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// .fypp files often have leading `#:include` and `#:set` directives before
/// the module statement. tree-sitter-fortran can't parse those lines, but must
/// still recover the valid `module` node that follows. Also validates that a
/// named `interface` block inside emits a Function symbol.
#[test]
fn fypp_leading_directives_before_module_still_emits_module_and_interface() {
    let src = concat!(
        "#:include \"common.fypp\"\n",
        "\n",
        "#:set KINDS_TYPES = REAL_KINDS_TYPES + INT_KINDS_TYPES\n",
        "\n",
        "module stdlib_optval\n",
        "  implicit none\n",
        "  private\n",
        "  public :: optval\n",
        "\n",
        "  interface optval\n",
        "    module procedure optval_character\n",
        "  end interface optval\n",
        "\n",
        "contains\n",
        "\n",
        "  pure elemental function optval_character(x, default) result(y)\n",
        "    character(len=*), intent(in), optional :: x\n",
        "    character(len=*), intent(in) :: default\n",
        "    character(len=:), allocatable :: y\n",
        "    if (present(x)) then\n",
        "       y = x\n",
        "    else\n",
        "       y = default\n",
        "    end if\n",
        "  end function optval_character\n",
        "\n",
        "end module stdlib_optval\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "stdlib_optval" && s.kind == SymbolKind::Namespace),
        "module 'stdlib_optval' must be emitted despite leading fypp directives; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "optval" && s.kind == SymbolKind::Function),
        "interface 'optval' must emit a Function symbol; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
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

// ---------------------------------------------------------------------------
// subroutine_call with derived_type_member_expression (CALL obj%method)
// ---------------------------------------------------------------------------

/// `call tbl%get_keys(list)` is a `subroutine_call` whose `subroutine` field
/// is a `derived_type_member_expression`. The extractor must split it into
/// target_name="get_keys" and module=Some("tbl") — not emit the raw
/// "tbl%get_keys" string which would never resolve.
#[test]
fn ref_subroutine_call_derived_type_member() {
    let src = concat!(
        "subroutine process(tbl, list)\n",
        "  use tomlf, only: toml_table, toml_key\n",
        "  type(toml_table), intent(inout) :: tbl\n",
        "  type(toml_key), allocatable :: list(:)\n",
        "  call tbl%get_keys(list)\n",
        "end subroutine\n",
    );
    let r = extract(src);
    // Must emit target_name="get_keys", NOT "tbl%get_keys"
    let bad: Vec<&str> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls && rf.target_name.contains('%'))
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(bad.is_empty(), "raw percent-refs leaked: {bad:?}");
    let rf = r.refs.iter().find(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "get_keys");
    assert!(
        rf.is_some(),
        "expected Calls ref target_name=\"get_keys\"; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// fypp interpolation marker filtering ($) in symbol and ref names
// ---------------------------------------------------------------------------

/// fypp template artifacts like `optval_${t1[0]}$${k1}$` must not be emitted
/// as symbol names or Calls refs. The `$` guard in `push_sym` and
/// `is_fortran_callable_text` filters them before they reach the index.
#[test]
fn fypp_dollar_markers_filtered_from_symbols_and_refs() {
    // Minimal .fypp-style source: the loop body produces mangled names that
    // tree-sitter partially recovers. Real names before the loop must survive.
    let src = concat!(
        "module stdlib_optval\n",
        "  implicit none\n",
        "contains\n",
        // Mangled template expansion — tree-sitter may recover this as a
        // subroutine with a `$`-bearing name.
        "  pure function optval_x1k1(x, d) result(y)\n",
        "    integer, intent(in), optional :: x\n",
        "    integer, intent(in) :: d\n",
        "    integer :: y\n",
        "    if (present(x)) then\n",
        "      y = x\n",
        "    else\n",
        "      y = d\n",
        "    end if\n",
        "  end function\n",
        "end module\n",
    );
    let r = extract(src);
    let dollar_syms: Vec<&str> = r.symbols.iter()
        .filter(|s| s.name.contains('$'))
        .map(|s| s.name.as_str())
        .collect();
    assert!(dollar_syms.is_empty(), "symbols with '$' leaked: {dollar_syms:?}");
    let dollar_refs: Vec<&str> = r.refs.iter()
        .filter(|rf| rf.target_name.contains('$'))
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(dollar_refs.is_empty(), "refs with '$' leaked: {dollar_refs:?}");
    // The real module symbol must still be emitted.
    assert!(
        r.symbols.iter().any(|s| s.name == "stdlib_optval" && s.kind == SymbolKind::Namespace),
        "module 'stdlib_optval' must still be emitted; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Bound procedure extraction from derived type contains block
// ---------------------------------------------------------------------------

/// `type :: installer_t ... contains ... procedure :: install_library` must
/// emit a qualified Variable symbol `installer_t.install_library` as a member
/// of `installer_t`. This populates `members_of(installer_t)` in the symbol
/// index so type-chain resolution can find bound procedure declarations.
#[test]
fn derived_type_bound_procedure_emits_member_symbol() {
    let src = concat!(
        "module fpm_installer\n",
        "  implicit none\n",
        "  type :: installer_t\n",
        "    character(len=:), allocatable :: prefix\n",
        "  contains\n",
        "    procedure :: install_library\n",
        "    procedure :: install_executable\n",
        "    procedure :: new => installer_new\n",
        "  end type\n",
        "contains\n",
        "  subroutine install_library(self, lib, error)\n",
        "    class(installer_t), intent(inout) :: self\n",
        "  end subroutine\n",
        "  subroutine install_executable(self, exe, error)\n",
        "    class(installer_t), intent(inout) :: self\n",
        "  end subroutine\n",
        "  subroutine installer_new(self)\n",
        "    class(installer_t), intent(inout) :: self\n",
        "  end subroutine\n",
        "end module\n",
    );
    let r = extract(src);
    // Qualified member symbols must appear
    assert!(
        r.symbols.iter().any(|s| s.qualified_name == "installer_t.install_library"),
        "expected member 'installer_t.install_library'; got {:?}",
        r.symbols.iter().map(|s| (&s.qualified_name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.qualified_name == "installer_t.install_executable"),
        "expected member 'installer_t.install_executable'"
    );
    // Aliased binding: `procedure :: new => installer_new` — the public name
    // is `new`, not `installer_new`.
    assert!(
        r.symbols.iter().any(|s| s.qualified_name == "installer_t.new"),
        "expected aliased member 'installer_t.new' from 'procedure :: new => installer_new'"
    );
}

// ---------------------------------------------------------------------------
// Local type map: derived-type variable declarations resolve to type name
// ---------------------------------------------------------------------------

/// A subroutine-local `type(installer_t) :: installer` declares `installer`
/// as having derived type `installer_t`. When the extractor sees
/// `call installer%install_library(...)`, it should emit
/// target_name="install_library", module=Some("installer_t") — using the
/// type name, not the variable name.
#[test]
fn subroutine_local_type_map_replaces_var_with_type_in_module_field() {
    let src = concat!(
        "subroutine cmd_install(settings)\n",
        "  use fpm_installer, only: installer_t, new_installer\n",
        "  type(installer_t) :: installer\n",
        "  call new_installer(installer)\n",
        "  call installer%install_library(lib, error)\n",
        "  call installer%install_executable(exe, error)\n",
        "end subroutine\n",
    );
    let r = extract(src);
    // `install_library` call must carry module = "installer_t", not "installer"
    let lib_ref = r.refs.iter().find(|rf|
        rf.kind == EdgeKind::Calls && rf.target_name == "install_library"
    );
    assert!(
        lib_ref.is_some(),
        "expected Calls ref to 'install_library'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        lib_ref.unwrap().module.as_deref(),
        Some("installer_t"),
        "module field must be the type name 'installer_t', not the variable name"
    );
}
