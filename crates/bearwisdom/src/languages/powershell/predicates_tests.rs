use super::is_powershell_builtin;

#[test]
fn matches_core_cmdlet_exact_case() {
    assert!(is_powershell_builtin("Write-Warning"));
    assert!(is_powershell_builtin("Select-Object"));
}

#[test]
fn matches_core_cmdlet_case_insensitively() {
    assert!(is_powershell_builtin("write-warning"));
    assert!(is_powershell_builtin("SELECT-OBJECT"));
    assert!(is_powershell_builtin("sElEcT-oBjEcT"));
}

#[test]
fn matches_reserved_keyword_and_primitive_type() {
    assert!(is_powershell_builtin("Try"));
    assert!(is_powershell_builtin("else"));
    assert!(is_powershell_builtin("hashtable"));
}

#[test]
fn rejects_project_shaped_names() {
    assert!(!is_powershell_builtin("Export-Excel"));
    assert!(!is_powershell_builtin("Get-TargetResource"));
}

#[test]
fn excludes_names_shadowed_by_project_declarations() {
    // These are real cmdlet names, but the corpus collision check found
    // project-local `function` declarations shadowing each one (Pester test
    // mocks / coverage instrumentation) — draining them corpus-wide would
    // short-circuit the same-file lookup rung for those shadowing sites.
    assert!(!is_powershell_builtin("Write-Host"));
    assert!(!is_powershell_builtin("Get-Process"));
    assert!(!is_powershell_builtin("Get-Service"));
    assert!(!is_powershell_builtin("Get-CimInstance"));
}
