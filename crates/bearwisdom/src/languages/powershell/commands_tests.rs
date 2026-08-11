use crate::languages::powershell::extract::extract;
use crate::types::EdgeKind;

#[test]
fn import_module_named_argument_extracts_bare_module_name() {
    let r = extract("Import-Module -Name 'DscResource.Test' -Force -ErrorAction 'Stop'");
    let imports: Vec<_> = r
        .refs
        .iter()
        .filter(|ref_| ref_.kind == EdgeKind::Imports)
        .collect();
    assert_eq!(
        imports.len(),
        1,
        "expected exactly one Imports ref; got {imports:?}"
    );
    assert_eq!(imports[0].target_name, "DscResource.Test");
}

#[test]
fn import_module_positional_argument_extracts_bare_module_name() {
    let r = extract("Import-Module PSScriptAnalyzer -PassThru -Force");
    let imports: Vec<_> = r
        .refs
        .iter()
        .filter(|ref_| ref_.kind == EdgeKind::Imports)
        .collect();
    assert_eq!(imports.len(), 1, "got {imports:?}");
    assert_eq!(imports[0].target_name, "PSScriptAnalyzer");
}

#[test]
fn import_module_dynamic_path_falls_back_to_calls() {
    // `$PSScriptRoot\..\..\ImportExcel.psd1` isn't a statically known module
    // name — the fallback keeps the command node covered without emitting a
    // garbage target_name built from the whole argument list.
    let r = extract("Import-Module $PSScriptRoot\\..\\..\\ImportExcel.psd1 -Force");
    assert!(
        r.refs
            .iter()
            .all(|ref_| ref_.kind != EdgeKind::Imports),
        "expected no Imports ref for a dynamic path; got {:?}",
        r.refs
    );
    assert!(
        r.refs
            .iter()
            .any(|ref_| ref_.kind == EdgeKind::Calls && ref_.target_name == "Import-Module"),
        "expected a Calls fallback ref; got {:?}",
        r.refs
    );
}

#[test]
fn import_module_computed_path_falls_back_to_calls() {
    let r = extract(
        "Import-Module -Name (Join-Path -Path $PSScriptRoot -ChildPath 'lib.psm1')",
    );
    assert!(
        r.refs
            .iter()
            .all(|ref_| ref_.kind != EdgeKind::Imports),
        "expected no Imports ref for a computed path; got {:?}",
        r.refs
    );
    assert!(r
        .refs
        .iter()
        .any(|ref_| ref_.kind == EdgeKind::Calls && ref_.target_name == "Import-Module"));
}

#[test]
fn dot_source_emits_imports_ref() {
    let r = extract(". $PSScriptRoot\\lib.ps1");
    assert!(
        r.refs
            .iter()
            .any(|ref_| ref_.kind == EdgeKind::Imports && ref_.target_name.contains("lib.ps1")),
        "expected an Imports ref covering the dot-sourced path; got {:?}",
        r.refs
    );
}

#[test]
fn dot_source_quoted_path_emits_imports_ref() {
    let r = extract(". \"$PSScriptRoot/lib.ps1\"");
    assert!(
        r.refs
            .iter()
            .any(|ref_| ref_.kind == EdgeKind::Imports),
        "expected an Imports ref; got {:?}",
        r.refs
    );
}

#[test]
fn call_operator_does_not_crash_and_emits_no_import() {
    // `&` invokes in a child scope — it doesn't bring anything into the
    // caller's scope, so it must not be mis-tagged as an Imports edge.
    let r = extract("& $scriptBlock");
    assert!(
        r.refs.iter().all(|ref_| ref_.kind != EdgeKind::Imports),
        "call operator should not emit an Imports ref; got {:?}",
        r.refs
    );
}
