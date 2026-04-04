// =============================================================================
// angular/coverage_tests.rs
//
// Node-kind coverage for AngularPlugin::symbol_node_kinds() and ref_node_kinds().
// Angular templates define no symbols of their own (empty symbol_node_kinds).
// All extractions are Calls refs produced by component/pipe/directive detection.
// =============================================================================

use super::extract;
use crate::types::EdgeKind;

// ---------------------------------------------------------------------------
// ref_node_kinds: element / self_closing_tag / pipe_call /
//                call_expression / interpolation / property_binding / event_binding
// ---------------------------------------------------------------------------

#[test]
fn cov_element_kebab_component_produces_calls() {
    // "element" with a hyphenated tag → Angular component selector → Calls
    let r = extract::extract("<app-header></app-header>", "test.component.html");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "AppHeader"),
        "kebab-case element should produce Calls(AppHeader); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_element_pascal_component_produces_calls() {
    // "element" with PascalCase tag → component usage → Calls
    let r = extract::extract("<UserCard></UserCard>", "test.component.html");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "UserCard"),
        "PascalCase element should produce Calls(UserCard); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_self_closing_tag_produces_calls() {
    // "self_closing_tag" with hyphenated name → Calls
    let r = extract::extract("<mat-icon />", "test.component.html");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "MatIcon"),
        "self-closing hyphenated tag should produce Calls(MatIcon); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_event_binding_produces_calls() {
    // "(click)=handler($event)" → Calls(handler) — event_binding node kind
    let r = extract::extract(
        r#"<button (click)="handleClick()">Click</button>"#,
        "btn.component.html",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "handleClick"),
        "event binding should produce Calls(handleClick); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_structural_directive_ngif_produces_calls() {
    // *ngIf → NgIfDirective (property_binding / interpolation path)
    let r = extract::extract(r#"<div *ngIf="show">Content</div>"#, "test.component.html");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name.contains("NgIf")),
        "*ngIf should produce Calls with NgIf in name; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
