// =============================================================================
// fortran/walk_tests.rs — Calls-ref emission for statement-shaped intrinsics
// =============================================================================

use crate::languages::fortran::extract::extract;
use crate::types::EdgeKind;

fn calls_targets(src: &str) -> Vec<String> {
    extract(src)
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.clone())
        .collect()
}

/// `DEALLOCATE(x)` has no dedicated grammar rule (unlike `ALLOCATE`), so it
/// parses through the same production as a bare function call. Emitting a
/// ref would create an unresolvable "call" to a statement keyword.
#[test]
fn bare_deallocate_emits_no_calls_ref() {
    let src = "subroutine foo(x)\n  integer, allocatable :: x(:)\n  deallocate(x)\nend subroutine";
    let targets = calls_targets(src);
    assert!(
        !targets.iter().any(|t| t.eq_ignore_ascii_case("deallocate")),
        "expected no Calls ref to 'deallocate'; got {targets:?}"
    );
}

/// `NULLIFY(p)` has the same grammar gap as `DEALLOCATE`.
#[test]
fn bare_nullify_emits_no_calls_ref() {
    let src = "subroutine foo(p)\n  integer, pointer :: p\n  nullify(p)\nend subroutine";
    let targets = calls_targets(src);
    assert!(
        !targets.iter().any(|t| t.eq_ignore_ascii_case("nullify")),
        "expected no Calls ref to 'nullify'; got {targets:?}"
    );
}

/// Fortran is case-insensitive; the suppression must not depend on source case.
#[test]
fn statement_keyword_suppression_is_case_insensitive() {
    let src = "subroutine foo(x)\n  integer, allocatable :: x(:)\n  DEALLOCATE(x)\nend subroutine";
    let targets = calls_targets(src);
    assert!(
        !targets.iter().any(|t| t.eq_ignore_ascii_case("deallocate")),
        "expected no Calls ref to 'DEALLOCATE'; got {targets:?}"
    );
}

/// `obj%deallocate()` is a real user-defined type-bound procedure call, not
/// the misparsed bare statement — the suppression must not reach it.
#[test]
fn member_call_named_deallocate_still_emits_ref() {
    let src = "subroutine foo(obj)\n  class(resource) :: obj\n  call obj%deallocate()\nend subroutine";
    let targets = calls_targets(src);
    assert!(
        targets.iter().any(|t| t.eq_ignore_ascii_case("deallocate")),
        "expected a Calls ref to member 'deallocate'; got {targets:?}"
    );
}

/// An ordinary subroutine call is unaffected by the statement-keyword gate.
#[test]
fn ordinary_call_still_emits_ref() {
    let src = "subroutine foo(x)\n  integer :: x\n  call bar(x)\nend subroutine";
    let targets = calls_targets(src);
    assert!(
        targets.iter().any(|t| t == "bar"),
        "expected a Calls ref to 'bar'; got {targets:?}"
    );
}

/// A genuine intrinsic call (`sum`) is still extracted as a Calls ref — the
/// statement-keyword gate is narrowly scoped to `deallocate`/`nullify` and
/// must not suppress real intrinsic-procedure calls left for `builtin_skip`
/// to drain.
#[test]
fn intrinsic_function_call_still_emits_ref() {
    let src = "function total(x)\n  integer :: x(:), total\n  total = sum(x)\nend function";
    let targets = calls_targets(src);
    assert!(
        targets.iter().any(|t| t == "sum"),
        "expected a Calls ref to 'sum'; got {targets:?}"
    );
}
