use super::*;

#[test]
fn interpolation_becomes_region() {
    let src = "<div>{{ userName }}</div>";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("userName")));
}

#[test]
fn property_binding_becomes_region() {
    let src = r#"<img [src]="avatarUrl" />"#;
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("avatarUrl")));
}

#[test]
fn event_binding_becomes_region() {
    let src = r#"<button (click)="handleClick($event)">Go</button>"#;
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("handleClick")));
}

#[test]
fn ng_for_becomes_for_of_region() {
    let src = r#"<li *ngFor="let user of users">{{user.name}}</li>"#;
    let regions = detect_regions(src);
    assert!(
        regions
            .iter()
            .any(|r| r.text.contains("for (let user of users)")),
        "got regions: {regions:#?}"
    );
}

#[test]
fn ng_if_becomes_region() {
    let src = r#"<div *ngIf="isActive">x</div>"#;
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("isActive")));
}

#[test]
fn empty_interpolation_skipped() {
    let src = "{{  }}";
    let regions = detect_regions(src);
    assert!(regions.is_empty());
}

#[test]
fn full_input_line_with_two_bindings_produces_two_regions() {
    let src = r#"<input [value]="formName" (input)="handleInput($event)" />"#;
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 2, "expected 2 regions, got {regions:#?}");
}

// --- $any cast unwrap ---------------------------------------------------------

#[test]
fn dollar_any_cast_unwrapped_in_interpolation() {
    // `$any(...)` is a no-op cast — the wrapped expression must remain but the
    // synthetic `$any` call must NOT appear in the emitted region (so the TS
    // extractor never emits a `$any` Calls ref).
    let src = "<div>{{ $any(user).name }}</div>";
    let regions = detect_regions(src);
    let text = &regions
        .iter()
        .find(|r| r.text.contains("user"))
        .expect("expected an interpolation region")
        .text;
    assert!(!text.contains("$any"), "got: {text}");
    assert!(text.contains("(user).name"), "got: {text}");
}

#[test]
fn dollar_any_cast_unwrapped_in_property_binding() {
    let src = r#"<app-grid [data]="$any(rows)" />"#;
    let regions = detect_regions(src);
    let text = &regions
        .iter()
        .find(|r| r.text.contains("rows"))
        .expect("expected a binding region")
        .text;
    assert!(!text.contains("$any"), "got: {text}");
    assert!(text.contains("(rows)"), "got: {text}");
}

#[test]
fn nested_dollar_any_fully_unwrapped() {
    let src = "<div>{{ $any($any(x)) }}</div>";
    let regions = detect_regions(src);
    let text = &regions
        .iter()
        .find(|r| r.text.contains('x'))
        .expect("expected a region")
        .text;
    assert!(!text.contains("$any"), "got: {text}");
}

// --- template reference variables ---------------------------------------------

#[test]
fn template_ref_var_collected() {
    let src = r#"<input #userInput type="text" />"#;
    let refs = collect_template_ref_vars(src);
    assert!(refs.contains(&"userInput".to_string()), "got: {refs:?}");
}

#[test]
fn template_ref_var_seeded_as_local_in_region() {
    // `#grid` declares a template-local; its use in a binding must resolve to a
    // seeded local, not emit a ref to a non-existent component member.
    let src = r#"<my-grid #grid></my-grid>
<button (click)="grid.refresh()">Refresh</button>"#;
    let regions = detect_regions(src);
    let text = &regions
        .iter()
        .find(|r| r.text.contains("grid.refresh"))
        .expect("expected an event-binding region")
        .text;
    assert!(text.contains("let grid: any;"), "got: {text}");
}

#[test]
fn no_template_ref_means_empty_prelude() {
    // Guard: a plain binding with no `#ref` declarations must NOT gain a prelude,
    // and legitimate identifiers still appear in the region.
    let src = r#"<button (click)="save()">Save</button>"#;
    let regions = detect_regions(src);
    let text = &regions
        .iter()
        .find(|r| r.text.contains("save()"))
        .expect("expected a binding region")
        .text;
    assert!(!text.contains("let "), "got: {text}");
    assert!(text.contains("save()"), "got: {text}");
}

#[test]
fn hash_inside_value_not_collected_as_ref() {
    // A `#` inside an attribute *value* (e.g. an href fragment) is not a template
    // reference variable — only a `#name` at an attribute boundary is.
    let src = r#"<a href="/page#section">link</a>"#;
    let refs = collect_template_ref_vars(src);
    assert!(refs.is_empty(), "got: {refs:?}");
}
