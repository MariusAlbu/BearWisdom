use super::is_nim_system_magic;

#[test]
fn nim_system_magics_decline() {
    // `system`-module intrinsics are language spec — they decline before the
    // ladder rather than binding to a same-named project symbol.
    assert!(is_nim_system_magic("echo"));
    assert!(is_nim_system_magic("len"));
    assert!(is_nim_system_magic("new"));
    assert!(is_nim_system_magic("high"));
    assert!(is_nim_system_magic("low"));
    assert!(is_nim_system_magic("repr"));
}

#[test]
fn nim_stdlib_module_names_do_not_decline() {
    // Stdlib MODULE names and ordinary stdlib procs resolve through the
    // externals path — they are not `system`-magic language constructs.
    assert!(!is_nim_system_magic("strutils"));
    assert!(!is_nim_system_magic("sequtils"));
    assert!(!is_nim_system_magic("jester"));
    assert!(!is_nim_system_magic("parseInt"));
}
