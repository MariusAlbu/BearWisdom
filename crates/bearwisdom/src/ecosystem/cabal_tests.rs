use super::{
    CabalEcosystem, Ecosystem, EcosystemKind, ExternalSourceLocator,
    GHC_BOOT_PACKAGES, find_haskell_cabal_get_deps_in_dir,
    parse_cabal_build_depends, path_to_haskell_module, shared_locator,
};
use std::sync::Arc;

#[test]
fn ecosystem_identity() {
    let c = CabalEcosystem;
    assert_eq!(c.id(), super::ID);
    assert_eq!(Ecosystem::kind(&c), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&c), &["haskell"]);
}

#[test]
fn legacy_locator_tag_is_haskell() {
    assert_eq!(ExternalSourceLocator::ecosystem(&CabalEcosystem), "haskell");
}

#[test]
fn haskell_parses_cabal_build_depends() {
    let tmp = std::env::temp_dir().join("bw-test-cabal-deps");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("test.cabal"), r#"
cabal-version: 2.0
name: test
version: 1.0
library
  build-depends:
    aeson >= 2.0,
    text,
    bytestring
"#).unwrap();
    let deps = parse_cabal_build_depends(&tmp);
    assert_eq!(deps, vec!["aeson", "bytestring", "text"]);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

#[test]
fn haskell_extracts_imports_with_qualified() {
    let mut out = std::collections::HashSet::new();
    super::extract_haskell_imports(
        "module Main where\nimport Data.List\nimport qualified Data.Map.Strict as M\nimport Control.Monad (when)\n",
        &mut out,
    );
    assert!(out.contains("Data.List"));
    assert!(out.contains("Data.Map.Strict"));
    assert!(out.contains("Control.Monad"));
}

#[test]
fn haskell_module_path_conversion() {
    assert_eq!(super::haskell_module_to_path_tail("Data.List"), Some("Data/List.hs".to_string()));
    assert_eq!(super::haskell_module_to_path_tail("Control.Monad.State"), Some("Control/Monad/State.hs".to_string()));
}

#[test]
fn ghc_boot_packages_get_full_walk() {
    // GHC boot packages must bypass narrowing. Narrowed walk uses requested_imports
    // to tail-match file paths, but ghc-internal uses GHC/Internal/Data/ prefixes
    // that don't match user imports like Data.Functor.
    assert!(GHC_BOOT_PACKAGES.contains(&"ghc-internal"));
    assert!(GHC_BOOT_PACKAGES.contains(&"ghc-prim"));
    assert!(GHC_BOOT_PACKAGES.contains(&"ghc-bignum"));
}

#[test]
fn cabal_get_returns_empty_for_missing_dir() {
    // Empty cabal-get directory yields no roots.
    let tmp = std::env::temp_dir().join("bw-test-cabal-get-empty");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let roots = find_haskell_cabal_get_deps_in_dir(&tmp, &["aeson".to_string()], &[]);
    assert!(roots.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn cabal_get_finds_src_subdir() {
    // When a cabal-get package has a `src/` subdirectory, the dep root points
    // to that subdirectory so module paths resolve without the `src/` prefix.
    let tmp = std::env::temp_dir().join("bw-test-cabal-get-src");
    let _ = std::fs::remove_dir_all(&tmp);
    let pkg = tmp.join("aeson-2.2.4.1");
    let src = pkg.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("Data.hs"), "module Data where\n").unwrap();
    let roots = find_haskell_cabal_get_deps_in_dir(&tmp, &["aeson".to_string()], &[]);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "aeson");
    assert_eq!(roots[0].root, src);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn cabal_get_falls_back_to_pkg_root_when_no_src() {
    // When no `src/` directory exists, the package root itself is used.
    let tmp = std::env::temp_dir().join("bw-test-cabal-get-nosrc");
    let _ = std::fs::remove_dir_all(&tmp);
    let pkg = tmp.join("hspec-2.11.17");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("Spec.hs"), "module Spec where\n").unwrap();
    let roots = find_haskell_cabal_get_deps_in_dir(&tmp, &["hspec".to_string()], &[]);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].root, pkg);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn path_to_haskell_module_strips_root_and_ext() {
    let tmp = std::env::temp_dir().join("bw-test-haskell-module");
    let root = tmp.join("src");
    let file = root.join("Test").join("Hspec").join("Core").join("Spec.hs");
    let module = path_to_haskell_module(&file, &root);
    assert_eq!(module, "Test.Hspec.Core.Spec");
}

#[test]
fn path_to_haskell_module_returns_empty_for_unrelated_path() {
    let root = std::path::PathBuf::from("/some/root");
    let file = std::path::PathBuf::from("/unrelated/path/Data/List.hs");
    let module = path_to_haskell_module(&file, &root);
    assert_eq!(module, "");
}

#[test]
fn path_to_haskell_module_single_component() {
    let tmp = std::env::temp_dir().join("bw-test-haskell-module-single");
    let root = tmp.clone();
    let file = root.join("Data.hs");
    let module = path_to_haskell_module(&file, &root);
    assert_eq!(module, "Data");
}

#[test]
fn cabal_get_does_not_match_package_name_prefix() {
    // `wai-extra-3.1.18` must NOT match the declared dep `wai`. The version
    // component after `<pkg>-` starts with a digit; anything else is a
    // different package whose name happens to share a prefix.
    let tmp = std::env::temp_dir().join("bw-test-cabal-get-prefix");
    let _ = std::fs::remove_dir_all(&tmp);
    // Create both `wai-3.2.1/` (correct match) and `wai-extra-3.1.18/` (should NOT match `wai`).
    let wai_pkg = tmp.join("wai-3.2.1");
    let wai_extra_pkg = tmp.join("wai-extra-3.1.18");
    std::fs::create_dir_all(&wai_pkg).unwrap();
    std::fs::create_dir_all(&wai_extra_pkg).unwrap();
    std::fs::write(wai_pkg.join("Wai.hs"), "module Network.Wai where\n").unwrap();
    std::fs::write(wai_extra_pkg.join("Test.hs"), "module Network.Wai.Test where\n").unwrap();
    let declared = vec!["wai".to_string()];
    let roots = find_haskell_cabal_get_deps_in_dir(&tmp, &declared, &[]);
    assert_eq!(roots.len(), 1, "exactly one root expected (wai, not wai-extra); got: {roots:?}");
    assert_eq!(roots[0].module_path, "wai");
    assert_eq!(roots[0].root, wai_pkg, "root should be wai-3.2.1, not wai-extra-3.1.18");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn scan_header_captures_class_method_operators() {
    // `(.=)` is a typeclass method operator defined inside a `class` body.
    // The symbol index must include it so bare-name resolution finds it in
    // external packages like `aeson`.
    let src = r#"
class KeyValue kv where
    (.=) :: ToJSON v => Key -> v -> kv
    infixr 8 .=
"#;
    let names = super::scan_haskell_header(src);
    assert!(
        names.iter().any(|n| n == ".="),
        "expected `.=` from class method signature; got: {names:?}"
    );
}

#[test]
fn scan_header_captures_record_field_names() {
    // Record field accessor names are generated functions visible to any
    // importer. They must appear in the symbol index alongside the type name.
    let src = r#"
data SResponse = SResponse
    { simpleStatus  :: H.Status
    , simpleHeaders :: H.ResponseHeaders
    , simpleBody    :: BSL.ByteString
    }
"#;
    let names = super::scan_haskell_header(src);
    assert!(names.iter().any(|n| n == "SResponse"), "expected SResponse type; got: {names:?}");
    assert!(names.iter().any(|n| n == "simpleStatus"), "expected simpleStatus field; got: {names:?}");
    assert!(names.iter().any(|n| n == "simpleHeaders"), "expected simpleHeaders field; got: {names:?}");
    assert!(names.iter().any(|n| n == "simpleBody"), "expected simpleBody field; got: {names:?}");
}

#[test]
fn scan_header_captures_data_constructors() {
    // Data constructors like `TagOpen`, `TagClose`, `Elt` must appear in the
    // symbol index alongside the type name so pattern-match references resolve.
    let src = r#"
data Tag str
    = TagOpen str [Attribute str]
    | TagClose str
    | TagText str
    deriving (Show, Eq)
"#;
    let names = super::scan_haskell_header(src);
    assert!(names.iter().any(|n| n == "Tag"), "expected Tag type; got: {names:?}");
    assert!(names.iter().any(|n| n == "TagOpen"), "expected TagOpen constructor; got: {names:?}");
    assert!(names.iter().any(|n| n == "TagClose"), "expected TagClose constructor; got: {names:?}");
    assert!(names.iter().any(|n| n == "TagText"), "expected TagText constructor; got: {names:?}");
}
