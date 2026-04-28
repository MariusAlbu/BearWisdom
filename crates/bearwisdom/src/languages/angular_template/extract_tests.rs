//! Tests for `angular_template::extract` — selector emission, attribute
//! directive detection, and selector-map lookup path.

use super::{extract, is_standard_html_element, kebab_to_pascal, normalize_attribute_as_directive};
use crate::types::EdgeKind;

// ---------------------------------------------------------------------------
// Existing basic tests (migrated from inline)
// ---------------------------------------------------------------------------

#[test]
fn file_stem_strips_component_suffix() {
    // file_stem is private — test through extract() host symbol name.
    let r = extract("<div></div>", "src/app/user.component.html");
    assert_eq!(r.symbols[0].name, "user");
    let r2 = extract("<div></div>", "foo.dialog.html");
    assert_eq!(r2.symbols[0].name, "foo");
}

#[test]
fn pascal_component_tag_becomes_calls_ref() {
    let src = r#"<div><UserCard name="x" /></div>"#;
    let r = extract(src, "parent.component.html");
    let calls: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert_eq!(calls, vec!["UserCard"]);
}

#[test]
fn kebab_tag_normalizes_to_pascal() {
    let src = "<app-user-card></app-user-card>";
    let r = extract(src, "parent.component.html");
    let calls: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert_eq!(calls, vec!["AppUserCard"]);
}

#[test]
fn html_builtins_not_emitted() {
    let src = "<div><p>text</p></div>";
    let r = extract(src, "x.component.html");
    assert!(r.refs.is_empty());
}

#[test]
fn ng_container_ignored_as_builtin() {
    let src = "<ng-container><p>x</p></ng-container>";
    let r = extract(src, "x.component.html");
    assert!(r.refs.is_empty());
}

// ---------------------------------------------------------------------------
// Raw-selector preserved in module field
// ---------------------------------------------------------------------------

#[test]
fn kebab_tag_module_stores_raw_selector() {
    let src = "<app-user-card></app-user-card>";
    let r = extract(src, "parent.component.html");
    let ref0 = r.refs.iter().find(|r| r.target_name == "AppUserCard").unwrap();
    // Raw kebab stored for resolver lookup.
    assert_eq!(ref0.module.as_deref(), Some("app-user-card"));
}

#[test]
fn pascal_tag_no_raw_selector() {
    // PascalCase tags from JSX-style usage already have the correct name.
    let src = r#"<UserCard></UserCard>"#;
    let r = extract(src, "parent.component.html");
    let ref0 = r.refs.iter().find(|r| r.target_name == "UserCard").unwrap();
    assert_eq!(ref0.module, None);
}

// ---------------------------------------------------------------------------
// Attribute directive detection
// ---------------------------------------------------------------------------

#[test]
fn structural_ngfor_emits_ref() {
    let src = r#"<div *ngFor="let x of xs"></div>"#;
    let r = extract(src, "parent.component.html");
    let names: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"ngFor"), "expected ngFor in {names:?}");
}

#[test]
fn structural_ngif_emits_ref() {
    let src = r#"<div *ngIf="show"></div>"#;
    let r = extract(src, "parent.component.html");
    let names: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"ngIf"), "expected ngIf in {names:?}");
}

#[test]
fn property_binding_ngclass_emits_ref() {
    let src = r#"<div [ngClass]="classes"></div>"#;
    let r = extract(src, "parent.component.html");
    let names: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"ngClass"), "expected ngClass in {names:?}");
}

#[test]
fn two_way_ngmodel_emits_ref() {
    let src = r#"<input [(ngModel)]="value" />"#;
    let r = extract(src, "parent.component.html");
    let names: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"ngModel"), "expected ngModel in {names:?}");
}

#[test]
fn event_binding_not_emitted() {
    // `(click)` is a DOM event, not an Angular directive.
    let src = r#"<button (click)="save()">Save</button>"#;
    let r = extract(src, "parent.component.html");
    // No ref for "click" — standard DOM event.
    assert!(
        !r.refs.iter().any(|r| r.target_name == "click"),
        "should not emit click event as directive ref"
    );
}

#[test]
fn plain_html_attr_not_emitted() {
    let src = r#"<a href="/home" class="nav">link</a>"#;
    let r = extract(src, "parent.component.html");
    assert!(r.refs.is_empty(), "plain HTML attrs should not emit refs");
}

#[test]
fn camel_case_attr_directive_emitted() {
    let src = r#"<div appHighlight></div>"#;
    let r = extract(src, "parent.component.html");
    let names: Vec<&str> = r.refs.iter().map(|r| r.target_name.as_str()).collect();
    assert!(names.contains(&"appHighlight"), "expected appHighlight in {names:?}");
}

#[test]
fn attribute_directive_module_stores_selector() {
    let src = r#"<div [appHighlight]="color"></div>"#;
    let r = extract(src, "parent.component.html");
    let ref0 = r.refs.iter().find(|r| r.target_name == "appHighlight");
    assert!(ref0.is_some(), "expected appHighlight ref");
    assert_eq!(ref0.unwrap().module.as_deref(), Some("appHighlight"));
}

// ---------------------------------------------------------------------------
// normalize_attribute_as_directive unit tests
// ---------------------------------------------------------------------------

#[test]
fn normalize_structural_directives() {
    assert_eq!(normalize_attribute_as_directive("*ngFor"), Some("ngFor".to_string()));
    assert_eq!(normalize_attribute_as_directive("*ngIf"), Some("ngIf".to_string()));
    assert_eq!(normalize_attribute_as_directive("*ngSwitchCase"), Some("ngSwitchCase".to_string()));
}

#[test]
fn normalize_property_bindings() {
    assert_eq!(normalize_attribute_as_directive("[ngClass]"), Some("ngClass".to_string()));
    assert_eq!(normalize_attribute_as_directive("[appHighlight]"), Some("appHighlight".to_string()));
    // Lowercase-only property binding not emitted (likely native).
    assert_eq!(normalize_attribute_as_directive("[class]"), None);
}

#[test]
fn normalize_two_way_bindings() {
    assert_eq!(normalize_attribute_as_directive("[(ngModel)]"), Some("ngModel".to_string()));
}

#[test]
fn normalize_event_bindings_skipped() {
    assert_eq!(normalize_attribute_as_directive("(click)"), None);
    assert_eq!(normalize_attribute_as_directive("(submit)"), None);
}

#[test]
fn normalize_plain_html_attrs_skipped() {
    assert_eq!(normalize_attribute_as_directive("href"), None);
    assert_eq!(normalize_attribute_as_directive("class"), None);
    assert_eq!(normalize_attribute_as_directive("id"), None);
    assert_eq!(normalize_attribute_as_directive("data-testid"), None);
}

// ---------------------------------------------------------------------------
// is_standard_html_element unit tests
// ---------------------------------------------------------------------------

#[test]
fn ng_pseudo_elements_are_standard() {
    assert!(is_standard_html_element("ng-template"));
    assert!(is_standard_html_element("ng-container"));
    assert!(is_standard_html_element("ng-content"));
}

#[test]
fn html5_elements_are_standard() {
    assert!(is_standard_html_element("div"));
    assert!(is_standard_html_element("span"));
    assert!(is_standard_html_element("input"));
    assert!(is_standard_html_element("p"));
    assert!(is_standard_html_element("h1"));
    assert!(is_standard_html_element("section"));
}

#[test]
fn kebab_elements_are_not_standard() {
    assert!(!is_standard_html_element("app-user-card"));
    assert!(!is_standard_html_element("router-outlet"));
    assert!(!is_standard_html_element("lib-button"));
}

#[test]
fn pascal_elements_are_not_standard() {
    assert!(!is_standard_html_element("UserCard"));
    assert!(!is_standard_html_element("AppComponent"));
}

// ---------------------------------------------------------------------------
// kebab_to_pascal unit tests
// ---------------------------------------------------------------------------

#[test]
fn kebab_to_pascal_basic() {
    assert_eq!(kebab_to_pascal("app-user-card"), "AppUserCard");
    assert_eq!(kebab_to_pascal("router-outlet"), "RouterOutlet");
    assert_eq!(kebab_to_pascal("lib-button"), "LibButton");
}

#[test]
fn kebab_to_pascal_single_word() {
    assert_eq!(kebab_to_pascal("app"), "App");
}

// ---------------------------------------------------------------------------
// Edge kinds
// ---------------------------------------------------------------------------

#[test]
fn component_refs_are_calls() {
    let src = "<app-header></app-header>";
    let r = extract(src, "x.component.html");
    assert!(r.refs.iter().all(|r| r.kind == EdgeKind::Calls));
}

#[test]
fn attribute_directive_refs_are_calls() {
    let src = r#"<div *ngFor="let x of xs"></div>"#;
    let r = extract(src, "x.component.html");
    let directive_refs: Vec<_> = r.refs.iter().filter(|r| r.target_name == "ngFor").collect();
    assert!(directive_refs.iter().all(|r| r.kind == EdgeKind::Calls));
}
