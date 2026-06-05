//! Tests for plain-HTML custom-element resolution.
//!
//! A custom-element tag in an HTML template binds to its component class only
//! through the scope-directed selector map (the same generic-engine binder
//! Angular/Vue use), keyed on real `customElements.define()` declarations. A
//! tag whose name happens to match an unrelated symbol but has no registered
//! selector must NOT bind — that coincidental-bind path is the soundness
//! property under test.

use super::profile::HTML_PROFILE;
use crate::indexer::resolve::engine::{
    FileContext, RefContext, Resolution, SymbolInfo, SymbolLookup,
};

/// Drive an HTML template ref through the generic engine ladder gated on
/// `HTML_PROFILE` — `selector_resolution` binds a custom-element `Calls` ref to
/// its defined class via the selector map.
fn run_resolve(
    file_ctx: &FileContext,
    ref_ctx: &RefContext<'_>,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    crate::type_checker::core::DefaultResolver {
        file_ctx,
        ref_ctx,
        lookup,
        kind_compatible: |_, _| true,
    }
    .resolve_all_with_profile(&HTML_PROFILE)
}

/// Minimal `SymbolLookup` stub for the selector-map path.
struct SelectorMapLookup {
    selectors: std::collections::HashMap<String, String>,
    symbols: Vec<SymbolInfo>,
}

impl SelectorMapLookup {
    fn new() -> Self {
        Self {
            selectors: std::collections::HashMap::new(),
            symbols: Vec::new(),
        }
    }

    fn with_selector(mut self, raw: &str, qname: &str) -> Self {
        self.selectors.insert(raw.to_string(), qname.to_string());
        self
    }

    fn with_symbol(mut self, id: i64, name: &str, qname: &str) -> Self {
        use std::sync::Arc;
        self.symbols.push(SymbolInfo {
            id,
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: "class".to_string(),
            visibility: Some("public".to_string()),
            file_path: Arc::from("src/app/user-card.ts"),
            scope_path: None,
            package_id: None,
            signature: None,
        });
        self
    }
}

impl SymbolLookup for SelectorMapLookup {
    fn by_name(&self, _name: &str) -> &[SymbolInfo] {
        &self.symbols
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.symbols.iter().find(|s| s.qualified_name == qname)
    }

    fn members_of(&self, _p: &str) -> &[SymbolInfo] { &[] }
    fn types_by_name(&self, _n: &str) -> &[SymbolInfo] { &[] }
    fn in_namespace(&self, _n: &str) -> Vec<&SymbolInfo> { vec![] }
    fn has_in_namespace(&self, _n: &str) -> bool { false }
    fn in_file(&self, _f: &str) -> &[SymbolInfo] { &[] }
    fn field_type_name(&self, _q: &str) -> Option<&str> { None }
    fn return_type_name(&self, _q: &str) -> Option<&str> { None }
    fn field_type_args(&self, _q: &str) -> Option<&[String]> { None }
    fn generic_params(&self, _n: &str) -> Option<&[String]> { None }
    fn reexports_from(&self, _f: &str) -> &[(String, String)] { &[] }
    fn is_external_name(&self, _n: &str, _l: &str) -> bool { false }

    fn selector_qname(&self, raw_selector: &str) -> Option<&str> {
        self.selectors.get(raw_selector).map(|s| s.as_str())
    }
}

fn host_symbol() -> crate::types::ExtractedSymbol {
    use crate::types::{SymbolKind, Visibility};
    crate::types::ExtractedSymbol {
        name: "page".to_string(),
        qualified_name: "page".to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn tag_ref(target: &str) -> crate::types::ExtractedRef {
    use crate::types::EdgeKind;
    crate::types::ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 5,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn html_file_ctx() -> FileContext {
    FileContext {
        file_path: "src/app/index.html".to_string(),
        language: "html".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    }
}

#[test]
fn defined_custom_element_binds_to_class() {
    // `customElements.define('user-card', UserCard)` was harvested elsewhere
    // into the selector map; the in-template `<user-card>` (PascalCase tag
    // emitted as `UserCard`) binds to the UserCard class via PascalToKebab.
    let lookup = SelectorMapLookup::new()
        .with_selector("user-card", "app.UserCard")
        .with_symbol(42, "UserCard", "app.UserCard");

    let host = host_symbol();
    let extracted = tag_ref("UserCard");
    let file_ctx = html_file_ctx();
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &host,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let resolution = run_resolve(&file_ctx, &ref_ctx, &lookup);
    assert!(resolution.is_some(), "defined custom element should resolve to its class");
    let res = resolution.unwrap();
    assert_eq!(res.target_symbol_id, 42);
    assert_eq!(res.strategy, "default_selector_map");
    assert!((res.confidence - 1.0).abs() < f64::EPSILON);
}

#[test]
fn undefined_library_tag_does_not_coincidentally_bind() {
    // A library custom element (`<ion-button>` → `IonButton`) has NO project
    // `customElements.define()`, so no selector is registered for it. Even with
    // an unrelated same-named `IonButton` symbol in the index, the tag must NOT
    // bind by name — the Invariant #2 guard the reverted by-name attempt failed.
    let lookup = SelectorMapLookup::new()
        .with_symbol(99, "IonButton", "app.IonButton");

    let host = host_symbol();
    let extracted = tag_ref("IonButton");
    let file_ctx = html_file_ctx();
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &host,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let resolution = run_resolve(&file_ctx, &ref_ctx, &lookup);
    assert!(
        resolution.is_none(),
        "library-named tag with no project define() must not bind by name, got {resolution:?}"
    );
}
