// =============================================================================
// types_companion_tests — @types packages stand in for their owners
// =============================================================================

use super::*;

#[test]
fn a_types_package_names_its_owner_scoped_or_not() {
    assert_eq!(owner_of_types_package("@types/react").as_deref(), Some("react"));
    assert_eq!(
        owner_of_types_package("@types/babel__core").as_deref(),
        Some("@babel/core")
    );
    assert_eq!(owner_of_types_package("react"), None);
    assert_eq!(owner_of_types_package("@types/"), None);
    assert_eq!(module_keys("@types/node"), vec!["@types/node", "node"]);
    assert_eq!(module_keys("rxjs"), vec!["rxjs"]);
}

#[test]
fn a_companion_supplies_the_owner_entry_only_when_the_owner_has_none() {
    let mut entries = HashMap::new();
    insert_package_entry(&mut entries, "@types/react", PathBuf::from("/nm/@types/react/index.d.ts"));
    assert_eq!(
        entries.get("react"),
        Some(&PathBuf::from("/nm/@types/react/index.d.ts"))
    );
    assert_eq!(
        entries.get("@types/react"),
        Some(&PathBuf::from("/nm/@types/react/index.d.ts"))
    );

    let mut entries = HashMap::new();
    insert_package_entry(&mut entries, "vue", PathBuf::from("/nm/vue/dist/vue.d.ts"));
    insert_package_entry(&mut entries, "@types/vue", PathBuf::from("/nm/@types/vue/index.d.ts"));
    assert_eq!(
        entries.get("vue"),
        Some(&PathBuf::from("/nm/vue/dist/vue.d.ts")),
        "a package that ships its own types keeps its own entry"
    );
}
