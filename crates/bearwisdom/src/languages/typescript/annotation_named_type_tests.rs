use crate::languages::typescript::extract;

/// The signature recorded for the named declarator in `src`.
fn declarator_signature(src: &str, name: &str) -> String {
    extract::extract(src, false)
        .symbols
        .iter()
        .find(|s| s.name == name)
        .and_then(|s| s.signature.clone())
        .unwrap_or_default()
}

#[test]
fn an_ambient_intersection_annotation_is_recorded_whole() {
    // `declare const v: A & B` carries BOTH arms' members; recording only the
    // first arm leaves every member of the second unresolvable.
    assert_eq!(
        declarator_signature("declare const fire: FireFunction & FireObject;", "fire"),
        "const fire: FireFunction & FireObject"
    );
}

#[test]
fn an_ambient_named_annotation_is_still_recorded() {
    assert_eq!(
        declarator_signature("declare const api: ApiType;", "api"),
        "const api: ApiType"
    );
}

#[test]
fn a_non_ambient_declarator_keeps_its_bare_signature() {
    // A non-ambient `const x: T = …` is typed by the flow seed and the
    // scope-qualified ref derivation, which an extractor-set type would
    // out-rank with an unqualified name.
    assert_eq!(
        declarator_signature("const api: ApiType = make();", "api"),
        "const api"
    );
}

#[test]
fn a_declaration_files_uninitialized_binding_records_its_annotation() {
    // A `.d.ts` writes `export const x: T` with no `declare` keyword and no
    // initializer — still ambient, and the annotation is the only type it has.
    assert_eq!(
        declarator_signature("export const fire: FireFunction & FireObject;", "fire"),
        "const fire: FireFunction & FireObject"
    );
}
