use super::*;

#[test]
fn rejects_non_class_bytes() {
    assert!(parse_class_file(b"not a class file").is_none());
}

#[test]
fn rejects_bad_magic() {
    let bad = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x34];
    assert!(parse_class_file(&bad).is_none());
}

#[test]
fn jvm_name_normalisation() {
    assert_eq!(jvm_name_to_dot("java/lang/String"), "java.lang.String");
    assert_eq!(jvm_short_name("java.lang.String"), "String");
    assert_eq!(jvm_short_name("Foo"), "Foo");
}

#[test]
fn supports_jar_and_aar() {
    assert!(supports_extension("jar"));
    assert!(supports_extension("aar"));
    assert!(!supports_extension("zip"));
}

#[test]
fn visibility_mapping() {
    assert!(matches!(visibility_for(ACC_PUBLIC), Visibility::Public));
    assert!(matches!(
        visibility_for(ACC_PROTECTED),
        Visibility::Protected
    ));
    assert!(matches!(visibility_for(ACC_PRIVATE), Visibility::Private));
    assert!(matches!(visibility_for(0), Visibility::Public));
}
