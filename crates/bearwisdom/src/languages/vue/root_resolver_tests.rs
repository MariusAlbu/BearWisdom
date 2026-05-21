// =============================================================================
// languages/vue/root_resolver_tests.rs — discovery shape regression coverage
// =============================================================================

use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use std::collections::HashMap;
use std::sync::Arc;

/// Minimal `SymbolLookup` fake: groups rows by `name` and by
/// `qualified_name` at construction so `by_name`/`by_qualified_name`
/// return slices into stable owned storage. Exercises only the two
/// methods `discover_component_instance` actually calls.
struct SyntheticLookup {
    by_name: HashMap<String, Vec<SymbolInfo>>,
    by_qname: HashMap<String, SymbolInfo>,
}

impl SyntheticLookup {
    fn new(rows: Vec<SymbolInfo>) -> Self {
        let mut by_name: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
        let mut by_qname: HashMap<String, SymbolInfo> = HashMap::new();
        for s in rows {
            by_name.entry(s.name.clone()).or_default().push(s.clone());
            by_qname.insert(s.qualified_name.clone(), s);
        }
        Self { by_name, by_qname }
    }
}

impl SymbolLookup for SyntheticLookup {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        self.by_name
            .get(name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.by_qname.get(qname)
    }

    fn members_of(&self, _: &str) -> &[SymbolInfo] { &[] }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] { &[] }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> { Vec::new() }
    fn has_in_namespace(&self, _: &str) -> bool { false }
    fn in_file(&self, _: &str) -> &[SymbolInfo] { &[] }
    fn field_type_name(&self, _: &str) -> Option<&str> { None }
    fn return_type_name(&self, _: &str) -> Option<&str> { None }
    fn field_type_args(&self, _: &str) -> Option<&[String]> { None }
    fn generic_params(&self, _: &str) -> Option<&[String]> { None }
    fn reexports_from(&self, _: &str) -> &[(String, String)] { &[] }
    fn is_external_name(&self, _: &str, _: &str) -> bool { false }
}

fn method(name: &str, qname: &str, file: &str) -> SymbolInfo {
    SymbolInfo {
        id: 0,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: "method".to_string(),
        visibility: None,
        file_path: Arc::from(file),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

#[test]
fn discovers_vue_2_component_instance() {
    let lookup = SyntheticLookup::new(vec![
        method("$emit", "vue.Vue.$emit", "ext:ts:vue/types/vue.d.ts"),
        method("$nextTick", "vue.Vue.$nextTick", "ext:ts:vue/types/vue.d.ts"),
        method(
            "$forceUpdate",
            "vue.Vue.$forceUpdate",
            "ext:ts:vue/types/vue.d.ts",
        ),
    ]);
    let result = super::discover_component_instance(&lookup);
    assert_eq!(result.as_deref(), Some("vue.Vue"));
}

#[test]
fn discovers_vue_3_component_public_instance() {
    let lookup = SyntheticLookup::new(vec![
        method(
            "$emit",
            "@vue/runtime-core.ComponentPublicInstance.$emit",
            "ext:ts:@vue/runtime-core/dist/runtime-core.d.ts",
        ),
        method(
            "$nextTick",
            "@vue/runtime-core.ComponentPublicInstance.$nextTick",
            "ext:ts:@vue/runtime-core/dist/runtime-core.d.ts",
        ),
        method(
            "$forceUpdate",
            "@vue/runtime-core.ComponentPublicInstance.$forceUpdate",
            "ext:ts:@vue/runtime-core/dist/runtime-core.d.ts",
        ),
    ]);
    let result = super::discover_component_instance(&lookup);
    assert_eq!(
        result.as_deref(),
        Some("@vue/runtime-core.ComponentPublicInstance")
    );
}

#[test]
fn discovery_unaffected_by_unrelated_dollar_emit_method() {
    // Some third-party package happens to declare a method named $emit
    // (a custom event bus). It must NOT be picked as the Vue instance
    // type because it lacks the rest of the canonical set.
    let lookup = SyntheticLookup::new(vec![
        method(
            "$emit",
            "vue.Vue.$emit",
            "ext:ts:vue/types/vue.d.ts",
        ),
        method(
            "$nextTick",
            "vue.Vue.$nextTick",
            "ext:ts:vue/types/vue.d.ts",
        ),
        method(
            "$forceUpdate",
            "vue.Vue.$forceUpdate",
            "ext:ts:vue/types/vue.d.ts",
        ),
        // Impostor: a same-named method on an unrelated type with no
        // siblings.
        method("$emit", "some-lib.Bus.$emit", "ext:ts:some-lib/index.d.ts"),
    ]);
    let result = super::discover_component_instance(&lookup);
    assert_eq!(
        result.as_deref(),
        Some("vue.Vue"),
        "discovery must reject candidates missing the canonical Vue method set"
    );
}

#[test]
fn no_vue_install_returns_none() {
    let lookup = SyntheticLookup::new(vec![method(
        "doSomething",
        "my-app.Helper.doSomething",
        "src/helper.ts",
    )]);
    let result = super::discover_component_instance(&lookup);
    assert!(result.is_none());
}
