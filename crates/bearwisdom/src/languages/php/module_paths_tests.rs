use super::*;

#[test]
fn well_formed_namespace_paths_are_bare_modules() {
    assert_eq!(
        (PHP_SOURCE_MODULE_PATH_POLICY.classify_specifier)("Illuminate\\Support"),
        ModuleSpecifierClass::Bare
    );
    assert_eq!(
        (PHP_SOURCE_MODULE_PATH_POLICY.classify_specifier)("\\PHPUnit\\Framework"),
        ModuleSpecifierClass::Bare
    );
    assert_eq!(
        (PHP_SOURCE_MODULE_PATH_POLICY.classify_specifier)("Illuminate\\\\Support"),
        ModuleSpecifierClass::Unsupported
    );
    assert_eq!(
        (PHP_SOURCE_MODULE_PATH_POLICY.classify_specifier)(""),
        ModuleSpecifierClass::Unsupported
    );
    assert!(!PHP_SOURCE_MODULE_PATH_POLICY.is_relative("Illuminate\\Support"));
}

#[test]
fn namespace_matches_its_psr4_directory_run() {
    let file = "ext:idx:F:/p/vendor/laravel/framework/src/Illuminate/Support/Arr.php";
    assert!(bare_module_matches_file(file, "Illuminate\\Support"));
    assert!(bare_module_matches_file(file, "\\Illuminate\\Support"));
    assert!(bare_module_matches_file(
        "vendor\\laravel\\framework\\src\\Illuminate\\Support\\Arr.php",
        "Illuminate\\Support"
    ));
}

#[test]
fn namespace_does_not_match_partial_segments_or_other_directories() {
    let file = "vendor/laravel/framework/src/Illuminate/Support/Arr.php";
    assert!(!bare_module_matches_file(file, "Illuminate\\Sup"));
    assert!(!bare_module_matches_file(file, "Support\\Arr"));
    assert!(!bare_module_matches_file(file, "Illuminate\\Database"));
    assert!(!bare_module_matches_file(file, ""));
    assert!(!bare_module_matches_file(file, "\\"));
}

#[test]
fn relative_candidates_are_never_produced() {
    assert!(
        (PHP_SOURCE_MODULE_PATH_POLICY.relative_candidate_paths)("Illuminate/Support").is_empty()
    );
}

#[test]
fn external_match_terms_use_the_namespace_leaf() {
    assert_eq!(
        external_import_match_terms("Illuminate\\Support"),
        vec!["support".to_string()]
    );
    assert_eq!(
        external_import_match_terms("\\Carbon"),
        vec!["carbon".to_string()]
    );
    assert!(external_import_match_terms("").is_empty());
    assert!(external_import_match_terms("Illuminate\\").is_empty());
}
