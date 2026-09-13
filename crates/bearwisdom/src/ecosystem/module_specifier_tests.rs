use super::{package_entry_key, workspace_package_id, workspace_package_sub_path};
use rustc_hash::FxHashMap;

fn declared(names: &[(&str, i64)]) -> FxHashMap<String, i64> {
    names
        .iter()
        .map(|(name, id)| ((*name).to_string(), *id))
        .collect()
}

#[test]
fn delegates_scoped_and_unscoped_external_package_keys() {
    assert_eq!(
        package_entry_key("ext:ts:@scope/pkg/dist/index.d.ts").as_deref(),
        Some("@scope/pkg")
    );
    assert_eq!(
        package_entry_key("ext:ruby:devise/lib/devise.rb").as_deref(),
        Some("devise")
    );
    assert_eq!(
        package_entry_key("ext:unknown:pkg/file"),
        None,
        "an unowned virtual-path scheme must not receive package semantics"
    );
}

#[test]
fn deep_specifier_splits_into_package_and_sub_path() {
    let names = declared(&[("package:core_client", 1)]);
    assert_eq!(
        workspace_package_sub_path("package:core_client/src/a.dart", &names),
        Some((1, "src/a.dart"))
    );
}

#[test]
fn bare_package_spelling_has_an_empty_sub_path() {
    let names = declared(&[("package:core_client", 1)]);
    assert_eq!(
        workspace_package_sub_path("package:core_client", &names),
        Some((1, ""))
    );
}

#[test]
fn longest_declared_name_claims_the_specifier() {
    let names = declared(&[("@org/utils", 1), ("@org/utils/inner", 2)]);
    assert_eq!(
        workspace_package_sub_path("@org/utils/inner/deep.ts", &names),
        Some((2, "deep.ts"))
    );
}

#[test]
fn undeclared_package_head_matches_nothing() {
    let names = declared(&[("package:core_client", 1)]);
    assert_eq!(workspace_package_sub_path("package:other/x.dart", &names), None);
    assert_eq!(workspace_package_id("package:other/x.dart", &names), None);
}

#[test]
fn package_id_agrees_with_the_sub_path_split() {
    let names = declared(&[("@org/utils", 7)]);
    assert_eq!(workspace_package_id("@org/utils/sub/mod", &names), Some(7));
    assert_eq!(workspace_package_id("@org/utils", &names), Some(7));
}
