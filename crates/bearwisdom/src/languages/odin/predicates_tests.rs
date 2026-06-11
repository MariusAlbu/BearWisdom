use super::is_builtin_collection_module;

#[test]
fn builtin_collections_decline() {
    // A collection-qualified module into a compiler-provided collection is
    // foreign — it declines before the ladder.
    assert!(is_builtin_collection_module("core:fmt"));
    assert!(is_builtin_collection_module("vendor:raylib"));
    assert!(is_builtin_collection_module("vendor:http"));
    assert!(is_builtin_collection_module("base:runtime"));
    assert!(is_builtin_collection_module("system:libc"));
}

#[test]
fn project_relative_modules_do_not_decline() {
    // Project imports are relative paths with no collection prefix — they stay
    // on the ladder so same-package references resolve.
    assert!(!is_builtin_collection_module("./shared"));
    assert!(!is_builtin_collection_module("utils"));
    assert!(!is_builtin_collection_module("game/entities"));
    // A non-builtin collection name is not a compiler collection.
    assert!(!is_builtin_collection_module("mylib:thing"));
}
