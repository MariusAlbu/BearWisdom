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

// ---------------------------------------------------------------------------
// pipe_call — interpolation {{ value | pipeName }} → Calls(<Name>Pipe)
// ---------------------------------------------------------------------------

#[test]
fn cov_pipe_call_in_interpolation_produces_calls() {
    // pipe_call: `value | date` in {{ }} expression → Calls(DatePipe)
    let r = extract::extract(
        "<p>{{ createdAt | date }}</p>",
        "item.component.html",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "DatePipe"),
        "pipe in interpolation should produce Calls(DatePipe); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_pipe_call_with_arguments_produces_calls() {
    // pipe_call with colon-separated args: `value | date:'short'` → Calls(DatePipe)
    let r = extract::extract(
        "<span>{{ ts | date:'short' }}</span>",
        "ts.component.html",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "DatePipe"),
        "pipe with args should produce Calls(DatePipe); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_pipe_call_chained_produces_multiple_calls() {
    // pipe_sequence: `value | uppercase | async` → Calls(UppercasePipe) + Calls(AsyncPipe)
    let r = extract::extract(
        "<p>{{ name | uppercase | async }}</p>",
        "name.component.html",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "UppercasePipe"),
        "chained pipe should produce Calls(UppercasePipe); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "AsyncPipe"),
        "chained pipe should produce Calls(AsyncPipe); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_pipe_call_in_property_binding_produces_calls() {
    // pipe in attribute value: [title]="value | translate" → Calls(TranslatePipe)
    let r = extract::extract(
        r#"<img [title]="label | translate" />"#,
        "img.component.html",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "TranslatePipe"),
        "pipe in property binding should produce Calls(TranslatePipe); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// structural_directive — *ngFor → NgForDirective Calls edge
// ---------------------------------------------------------------------------

#[test]
fn cov_structural_directive_ngfor_produces_calls() {
    // *ngFor → NgForDirective Calls edge
    let r = extract::extract(
        r#"<li *ngFor="let item of items">{{ item }}</li>"#,
        "list.component.html",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name.contains("NgFor")),
        "*ngFor should produce Calls with NgFor in name; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_structural_directive_ngswitch_produces_calls() {
    // *ngSwitch → NgSwitchDirective Calls edge
    let r = extract::extract(
        r#"<div [ngSwitch]="view"><span *ngSwitchCase="'a'">A</span></div>"#,
        "switch.component.html",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name.contains("NgSwitch")),
        "*ngSwitchCase should produce Calls with NgSwitch; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// event_binding — on- prefix variant
// ---------------------------------------------------------------------------

#[test]
fn cov_event_binding_on_prefix_produces_calls() {
    // Angular long-form event binding: on-click="handler()" → Calls(handler)
    let r = extract::extract(
        r#"<button on-click="save()">Save</button>"#,
        "form.component.html",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "save"),
        "on- prefix event binding should produce Calls(save); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// template sentinel symbol — Class symbol created from file stem
// ---------------------------------------------------------------------------

#[test]
fn cov_template_sentinel_symbol_uses_file_stem() {
    // The template file itself should emit a Class symbol named from the stem
    let r = extract::extract("<div>Hello</div>", "user-profile.component.html");
    assert!(
        r.symbols.iter().any(|s| s.name == "UserProfile"),
        "template sentinel should be named UserProfile from stem; got: {:?}",
        r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// standard HTML — no spurious Calls edges for lowercase tags
// ---------------------------------------------------------------------------

#[test]
fn cov_lowercase_html_tags_do_not_produce_calls() {
    // Plain HTML elements must not produce Calls edges
    let r = extract::extract(
        "<div><p><span><a href=\"#\">link</a></span></p></div>",
        "plain.component.html",
    );
    let html_calls: Vec<_> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .filter(|rf| matches!(rf.target_name.as_str(), "Div" | "P" | "Span" | "A" | "div" | "p" | "span" | "a"))
        .collect();
    assert!(
        html_calls.is_empty(),
        "standard HTML tags must not produce Calls; got: {:?}",
        html_calls.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

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
